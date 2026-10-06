// cdev for VSCodium / VS Code — a thin client over the cdev sidecar.
// No dependencies, no build step. It starts (or attaches to) `cdev`, listens
// to its event stream, and shows values next to your code.
'use strict';

const vscode = require('vscode');
const cp = require('child_process');
const http = require('http');
const path = require('path');
const { lineText, locKey, fnMarkdown } = require('./format');

let proc = null;          // cdev child process we started (null when attached / idle)
let root = null;          // cdev's working directory: `loc` paths are relative to it
let stream = null;        // active SSE request
let reconnect = null;
let panel = null;
let status;
let output;
let inlineOn = true;
// absolute file → Map(line → { text, events: [] })
const values = new Map();
const lastByName = new Map(); // watch name → previous value (to show what changed)
const STATIC_LANGS = ['javascript', 'typescript', 'typescriptreact', 'javascriptreact'];
// document uri → { version, promise of the static report, good: last report that parsed }
const reports = new Map();

const valueDeco = vscode.window.createTextEditorDecorationType({
  after: { margin: '0 0 0 2.5em', color: new vscode.ThemeColor('editorCodeLens.foreground'), fontStyle: 'italic' },
  rangeBehavior: vscode.DecorationRangeBehavior.ClosedClosed,
});
const stepDeco = vscode.window.createTextEditorDecorationType({
  isWholeLine: true,
  backgroundColor: new vscode.ThemeColor('editor.findMatchHighlightBackground'),
  overviewRulerColor: new vscode.ThemeColor('editorOverviewRuler.findMatchForeground'),
  overviewRulerLane: vscode.OverviewRulerLane.Center,
});

const cfg = () => vscode.workspace.getConfiguration('cdev');
const port = () => cfg().get('port') || 4400;
const workspaceRoot = () => (vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0].uri.fsPath) || process.cwd();

function activate(context) {
  output = vscode.window.createOutputChannel('cdev');
  status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 50);
  status.command = 'cdev.showPanel';
  setStatus('idle');
  status.show();
  inlineOn = cfg().get('inlineValues') !== false;

  const cmd = (name, fn) => context.subscriptions.push(vscode.commands.registerCommand(name, fn));
  cmd('cdev.autoWatchFile', (uri, fnName) => startForFile(uri, true, fnName));
  cmd('cdev.watchFile', (uri) => startForFile(uri, false));
  cmd('cdev.runCommand', async () => {
    const line = await vscode.window.showInputBox({ prompt: 'Command to run under cdev', placeHolder: 'npm run dev', value: 'npm run dev' });
    if (line) start(['run', '--', 'sh', '-c', line], workspaceRoot());
  });
  cmd('cdev.attach', () => { root = workspaceRoot(); connect(); showPanel(); });
  cmd('cdev.showPanel', showPanel);
  cmd('cdev.stop', stop);
  cmd('cdev.toggleInline', () => { inlineOn = !inlineOn; refresh(); });
  cmd('cdev.clear', () => { values.clear(); lastByName.clear(); refresh(); post('/api/clear'); });
  cmd('cdev.explainFile', explainFile);

  context.subscriptions.push(
    vscode.languages.registerHoverProvider({ scheme: 'file' }, { provideHover }),
    vscode.languages.registerHoverProvider(STATIC_LANGS.map((language) => ({ language })), { provideHover: provideStaticHover }),
    vscode.workspace.onDidCloseTextDocument((doc) => reports.delete(doc.uri.toString())),
    vscode.languages.registerCodeLensProvider(
      [{ language: 'python' }, { language: 'javascript' }, { language: 'typescript' }, { language: 'typescriptreact' }, { language: 'javascriptreact' }],
      { provideCodeLenses },
    ),
    vscode.window.onDidChangeVisibleTextEditors(refresh),
    vscode.workspace.onDidChangeConfiguration((e) => { if (e.affectsConfiguration('cdev')) { inlineOn = cfg().get('inlineValues') !== false; refresh(); } }),
    { dispose: stop },
    output, status, valueDeco, stepDeco,
  );

  // a cdev already running (e.g. started in a terminal)? attach quietly
  probe().then((up) => { if (up) { root = workspaceRoot(); connect(); } });
}

function deactivate() { stop(); }

// ---------------------------------------------------------------- process

function startForFile(uri, auto, fnName) {
  const file = (uri && uri.fsPath) || (vscode.window.activeTextEditor && vscode.window.activeTextEditor.document.uri.fsPath);
  if (!file) return vscode.window.showWarningMessage('cdev: open a file first');
  const args = [];
  if (auto) args.push('--auto');
  if (auto && typeof fnName === 'string') args.push('--only', fnName);
  args.push('watch', file);
  start(args, workspaceRoot());
}

function start(args, cwd) {
  stop();
  values.clear();
  lastByName.clear();
  refresh();
  root = cwd;
  const full = ['--headless', '--port', String(port()), ...args];
  output.appendLine(`$ ${cfg().get('path')} ${full.join(' ')}`);
  proc = cp.spawn(cfg().get('path') || 'cdev', full, { cwd, env: process.env });
  proc.stdout.on('data', (d) => output.append(d.toString()));
  proc.stderr.on('data', (d) => output.append(d.toString()));
  proc.on('error', (e) => {
    vscode.window.showErrorMessage(`cdev: could not start "${cfg().get('path')}" (${e.message}). Set cdev.path or run \`cargo install --path .\``);
    setStatus('idle');
  });
  proc.on('exit', (code) => { output.appendLine(`cdev exited (${code})`); proc = null; setStatus('idle'); });
  setStatus('starting');
  setTimeout(() => { connect(); showPanel(); }, 700);
}

function stop() {
  if (stream) { stream.destroy(); stream = null; }
  clearTimeout(reconnect);
  if (proc) { proc.kill('SIGINT'); proc = null; }
  setStatus('idle');
}

function setStatus(s) {
  const text = { idle: '$(eye-closed) cdev', starting: '$(sync~spin) cdev', live: '$(eye) cdev' }[s];
  status.text = text;
  status.tooltip = s === 'live' ? `cdev on :${port()} — click for the panel` : 'cdev — run "cdev: Auto-watch this file"';
}

function probe() {
  return new Promise((resolve) => {
    const req = http.get({ host: '127.0.0.1', port: port(), path: '/health', timeout: 800 }, (res) => { res.resume(); resolve(res.statusCode === 200); });
    req.on('error', () => resolve(false));
    req.on('timeout', () => { req.destroy(); resolve(false); });
  });
}

function post(p) {
  const req = http.request({ host: '127.0.0.1', port: port(), path: p, method: 'POST' }, (res) => res.resume());
  req.on('error', () => {});
  req.end();
}

// ---------------------------------------------------------------- event stream

function connect() {
  if (stream) stream.destroy();
  // prime with what's already there, then follow the live stream
  http.get({ host: '127.0.0.1', port: port(), path: '/api/events?kind=watch&limit=3000' }, (res) => {
    let body = '';
    res.on('data', (d) => (body += d));
    res.on('end', () => { try { JSON.parse(body).forEach(onEvent); refresh(); } catch { /* ignore */ } });
  }).on('error', () => {});

  stream = http.get({ host: '127.0.0.1', port: port(), path: '/stream', headers: { accept: 'text/event-stream' } }, (res) => {
    setStatus('live');
    let buf = '';
    res.setEncoding('utf8');
    res.on('data', (chunk) => {
      buf += chunk;
      let i;
      while ((i = buf.indexOf('\n\n')) >= 0) {
        const block = buf.slice(0, i);
        buf = buf.slice(i + 2);
        const data = block.split('\n').filter((l) => l.startsWith('data:')).map((l) => l.slice(5).trim()).join('');
        if (data) { try { onEvent(JSON.parse(data)); scheduleRefresh(); } catch { /* ignore */ } }
      }
    });
    res.on('end', retry);
  });
  stream.on('error', retry);
}

function retry() {
  stream = null;
  setStatus(proc ? 'starting' : 'idle');
  clearTimeout(reconnect);
  if (proc) reconnect = setTimeout(connect, 1000);
}

function toAbs(loc) {
  const m = /^(.*):(\d+)$/.exec(loc || '');
  if (!m) return null;
  const file = path.isAbsolute(m[1]) ? m[1] : path.join(root || workspaceRoot(), m[1]);
  return { file, line: +m[2] - 1 };
}

function onEvent(e) {
  if (e.kind === '_clear') { values.clear(); lastByName.clear(); return; }
  if (e.kind !== 'watch' || !e.loc) return;
  const at = toAbs(e.loc);
  if (!at) return;
  const prev = lastByName.get(e.name);
  lastByName.set(e.name, e.v);
  if (!values.has(at.file)) values.set(at.file, new Map());
  const lines = values.get(at.file);
  const entry = lines.get(at.line) || { text: '', events: [] };
  // several watches can land on one line (frame + containers): keep one text per name
  entry.byName = entry.byName || new Map();
  entry.byName.set(e.name, lineText(e, prev));
  entry.text = [...entry.byName.values()].join('   ');
  entry.events.push(e);
  if (entry.events.length > 30) entry.events.shift();
  lines.set(at.line, entry);
}

let pending = null;
function scheduleRefresh() {
  if (pending) return;
  pending = setTimeout(() => { pending = null; refresh(); }, 120);
}

function refresh() {
  for (const ed of vscode.window.visibleTextEditors) {
    const lines = values.get(ed.document.uri.fsPath);
    if (!inlineOn || !lines) { ed.setDecorations(valueDeco, []); continue; }
    const decos = [];
    for (const [line, entry] of lines) {
      if (line >= ed.document.lineCount) continue;
      const end = ed.document.lineAt(line).range.end;
      const text = entry.text.length > 140 ? entry.text.slice(0, 139) + '…' : entry.text;
      decos.push({ range: new vscode.Range(end, end), renderOptions: { after: { contentText: '  ' + text } } });
    }
    ed.setDecorations(valueDeco, decos);
  }
}

// ---------------------------------------------------------------- hover + codelens

function provideHover(doc, pos) {
  const lines = values.get(doc.uri.fsPath);
  const entry = lines && lines.get(pos.line);
  if (!entry) return null;
  const md = new vscode.MarkdownString();
  const last = entry.events[entry.events.length - 1];
  md.appendMarkdown(`**cdev** · \`${last.name}\`${last.shape ? ` · \`${last.shape}\`` : ''}${last.ds ? ` · ${last.ds}` : ''}\n\n`);
  md.appendCodeblock(JSON.stringify(last.v, null, 2).slice(0, 2000), 'json');
  const hist = entry.events.filter((e) => e.name === last.name).slice(-6, -1).reverse();
  if (hist.length) {
    md.appendMarkdown('\n\nearlier:\n');
    for (const h of hist) md.appendMarkdown(`\n- \`${JSON.stringify(h.v).slice(0, 110)}\``);
  }
  md.appendMarkdown('\n\n[Open panel](command:cdev.showPanel)');
  md.isTrusted = true;
  return new vscode.Hover(md);
}

const FN_RE = [
  /^\s*(?:async\s+)?def\s+([A-Za-z_]\w*)\s*\(/,                                         // python
  /^\s*(?:export\s+)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][\w$]*)\s*[(<]/,           // function f(
  /^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?(?:function|\([^)]*\)\s*(?::[^=]+)?=>|[A-Za-z_$][\w$]*\s*=>)/, // const f = (...) =>
];

async function provideCodeLenses(doc) {
  if (cfg().get('codeLens') === false) return [];
  const lenses = [];
  // what each function does, read from the source (nothing runs)
  const r = cfg().get('staticLens') === false ? null : await report(doc);
  for (const f of r ? r.fns : []) {
    if (f.kind === 'module') continue;
    const range = new vscode.Range(f.line - 1, 0, f.line - 1, 0);
    lenses.push(new vscode.CodeLens(range, { title: f.lens, tooltip: f.summary, command: 'cdev.explainFile', arguments: [doc.uri, f.signature] }));
  }
  for (let i = 0; i < doc.lineCount; i++) {
    const text = doc.lineAt(i).text;
    for (const re of FN_RE) {
      const m = re.exec(text);
      if (m) {
        const range = new vscode.Range(i, 0, i, 0);
        lenses.push(new vscode.CodeLens(range, { title: `▶ auto-watch ${m[1]}`, command: 'cdev.autoWatchFile', arguments: [doc.uri, m[1]] }));
        break;
      }
    }
  }
  return lenses;
}

// ---------------------------------------------------------------- static view

// Runs `cdev explain` on the editor's text (saved or not). The file is read, never run,
// and no sidecar is needed.
function explain(doc, flag) {
  return new Promise((resolve) => {
    const child = cp.spawn(cfg().get('path') || 'cdev', ['explain', flag, '--name', doc.uri.fsPath]);
    let out = '';
    child.stdout.on('data', (d) => (out += d));
    child.on('error', () => resolve(null));
    child.on('close', (code) => resolve(code === 0 ? out : null));
    child.stdin.on('error', () => {});
    child.stdin.end(doc.getText());
  });
}

// The static report for a document, analysed once per edit. While the text doesn't
// parse (mid-typing), the last report that did is kept.
function report(doc) {
  if (!STATIC_LANGS.includes(doc.languageId)) return Promise.resolve(null);
  const key = doc.uri.toString();
  const hit = reports.get(key);
  if (hit && hit.version === doc.version) return hit.promise;
  const entry = { version: doc.version, good: hit && hit.good };
  entry.promise = explain(doc, '--json').then((out) => {
    try { if (out) entry.good = JSON.parse(out); } catch { /* keep the last good one */ }
    return entry.good || null;
  });
  reports.set(key, entry);
  return entry.promise;
}

async function provideStaticHover(doc, pos) {
  const r = await report(doc);
  const f = r && r.fns.find((x) => x.kind !== 'module' && x.line === pos.line + 1);
  return f ? new vscode.Hover(new vscode.MarkdownString(fnMarkdown(f))) : null;
}

// Opens the whole report beside the code; from a CodeLens, scrolled to that function.
async function explainFile(uri, signature) {
  const active = vscode.window.activeTextEditor && vscode.window.activeTextEditor.document;
  const doc = uri && uri.fsPath ? await vscode.workspace.openTextDocument(uri) : active;
  if (!doc) return vscode.window.showWarningMessage('cdev: open a file first');
  const text = await explain(doc, '--text');
  if (text === null) return vscode.window.showWarningMessage('cdev: could not read this file (JS/TS only for now, and it has to parse)');
  const out = await vscode.workspace.openTextDocument({ content: text, language: 'plaintext' });
  const ed = await vscode.window.showTextDocument(out, { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true, preview: true });
  const at = typeof signature === 'string' ? text.split('\n').findIndex((l) => l.trimStart().startsWith(signature)) : -1;
  if (at >= 0) ed.revealRange(new vscode.Range(at, 0, at, 0), vscode.TextEditorRevealType.AtTop);
}

// ---------------------------------------------------------------- panel

function showPanel() {
  if (panel) { panel.reveal(vscode.ViewColumn.Beside, true); return; }
  panel = vscode.window.createWebviewPanel('cdev', 'cdev', { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true }, { enableScripts: true, retainContextWhenHidden: true });
  const url = `http://127.0.0.1:${port()}/`;
  panel.webview.html = `<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; frame-src http://127.0.0.1:* http://localhost:*; script-src 'unsafe-inline'; style-src 'unsafe-inline';">
<style>html,body,iframe{margin:0;padding:0;border:0;width:100%;height:100%;overflow:hidden;background:transparent}</style></head>
<body><iframe src="${url}"></iframe>
<script>
  const vscode = acquireVsCodeApi();
  // the panel (iframe) asks the editor to open / highlight source locations
  window.addEventListener('message', (e) => { if (e.data && e.data.cdev) vscode.postMessage(e.data); });
</script></body></html>`;
  panel.webview.onDidReceiveMessage((m) => {
    const at = toAbs(m.loc);
    if (!at) return;
    const uri = vscode.Uri.file(at.file);
    vscode.window.showTextDocument(uri, { viewColumn: vscode.ViewColumn.One, preserveFocus: m.type === 'step', preview: true }).then((ed) => {
      const range = new vscode.Range(at.line, 0, at.line, 0);
      ed.revealRange(range, vscode.TextEditorRevealType.InCenterIfOutsideViewport);
      if (m.type === 'step') ed.setDecorations(stepDeco, [range]);
      else ed.selection = new vscode.Selection(range.start, range.start);
    }, () => {});
  });
  panel.onDidDispose(() => {
    panel = null;
    for (const ed of vscode.window.visibleTextEditors) ed.setDecorations(stepDeco, []);
  });
}

module.exports = { activate, deactivate, locKey };
