// Pure helpers (no `vscode` import) so they can be tested with plain node.
'use strict';

const clean = (v) => JSON.stringify(v, (k, x) => (k === '__class' || k === '__id' ? undefined : x));

function short(v, max = 40) {
  let s;
  if (v === null || typeof v !== 'object') s = typeof v === 'string' ? JSON.stringify(v) : String(v);
  else if (v.__t === 'undefined') s = 'undefined';
  else if (v.__t === 'function') s = 'ƒ ' + (v.name || '');
  else if (v.__t && v.v !== undefined) s = `${v.__t}(${v.v})`;
  else s = clean(v);
  return s.length > max ? s.slice(0, max - 1) + '…' : s;
}

/** Text shown at the end of a line for one watch event.
 *  `fn()` frame snapshots show only what changed since the previous snapshot. */
function lineText(e, prev) {
  if (e.name.endsWith('()') && e.v && typeof e.v === 'object' && !Array.isArray(e.v)) {
    const keys = Object.keys(e.v).filter((k) => !prev || typeof prev !== 'object' || JSON.stringify(prev[k]) !== JSON.stringify(e.v[k]));
    const shown = keys.length ? keys : Object.keys(e.v);
    return shown.map((k) => `${k}=${short(e.v[k])}`).join('  ');
  }
  const dot = e.auto && e.name.lastIndexOf('.');
  const label = dot > 0 ? e.name.slice(dot + 1) : e.name;
  return `${label} = ${short(e.v, 60)}`;
}

const locKey = (file, line) => `${file}:${line}`;

/** Hover text for one function of a `cdev explain --json` report. */
function fnMarkdown(f) {
  const rows = (f.rows || []).map(([label, text]) => `- **${label}** \`${String(text).replace(/`/g, "'")}\``).join('\n');
  return `**cdev · static** \`${f.signature}\` · lines ${f.line}–${f.end}\n\n${f.summary}\n\n${rows}`;
}

module.exports = { lineText, short, locKey, fnMarkdown };
