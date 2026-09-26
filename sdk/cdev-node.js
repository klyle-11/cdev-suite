/* cdev Node SDK — zero dependencies, CommonJS.
 *
 *   node --require ./cdev-node.js app.js      (cdev run does this for you)
 *   const cdev = require('./cdev-node.js')    (or use the injected global `cdev`)
 *
 * cdev.w(value, label?)   record a value (+ inferred type) and return it unchanged
 * cdev.trace(fn, name?)   wrap a function: args, return value, errors, duration, caller
 * cdev.traceAll(objOrClass)  trace every method
 * cdev.log(...args)       explicit log event
 *
 * Auto (when loaded via --require): console.*, uncaught errors, incoming
 * http(s) server requests, outgoing http(s)/fetch requests, listening ports.
 * Disable with CDEV_OFF=1.
 */
'use strict';

(function () {
  if (globalThis.__cdev) { module.exports = globalThis.__cdev; return; }

  const noop = (v) => v;
  if (process.env.CDEV_OFF) {
    const off = { w: noop, watch: noop, trace: noop, traceAll: noop, log() {}, send() {}, flush() {} };
    globalThis.__cdev = globalThis.cdev = off;
    module.exports = off;
    return;
  }

  const http = require('http');
  const https = require('https');
  const util = require('util');
  const path = require('path');
  const fs = require('fs');
  const { AsyncLocalStorage } = require('async_hooks');

  const TARGET = new URL(process.env.CDEV_URL || 'http://127.0.0.1:4400');
  const rawRequest = http.request; // captured before patching; used for our own transport
  const SVC = process.env.CDEV_SERVICE || pkgName() || path.basename(process.cwd());
  const MAX_BODY = 16 * 1024;
  const IGNORE = new RegExp(process.env.CDEV_IGNORE ||
    '^/(@vite|@fs|@id|@react-refresh|__vite|_next/|__nextjs|node_modules/|favicon\\.ico)|\\.(m?js|jsx|tsx?|css|map|png|jpe?g|gif|svg|ico|woff2?|ttf|webp)(\\?|$)|[?&](import|t|v)=');
  const REDACT = /^(authorization|cookie|set-cookie|x-api-key|proxy-authorization)$/i;
  const als = new AsyncLocalStorage();
  const cwd = process.cwd();

  function pkgName() {
    try { return JSON.parse(fs.readFileSync(path.join(process.cwd(), 'package.json'), 'utf8')).name; } catch { return ''; }
  }

  // ---------- transport ----------
  let queue = [];
  let timer = null;
  let downUntil = 0;

  function send(ev) {
    ev.ts = ev.ts || Date.now();
    ev.svc = ev.svc || SVC;
    ev.lang = ev.lang || 'node';
    ev.pid = process.pid;
    queue.push(ev);
    if (queue.length > 5000) queue.shift();
    if (!timer) {
      timer = setTimeout(() => flush(), 40);
      if (timer.unref) timer.unref();
    }
  }

  // keepAlive: hold the process open until the batch is sent (used at the end of short scripts)
  function flush(keepAlive = false) {
    if (timer) { clearTimeout(timer); timer = null; }
    if (!queue.length) return;
    const batch = queue;
    queue = [];
    if (Date.now() < downUntil) return;
    let body;
    try { body = JSON.stringify(batch); } catch { return; }
    try {
      const req = rawRequest({
        hostname: TARGET.hostname, port: TARGET.port || 80, path: '/ingest', method: 'POST', agent: false,
        headers: { 'content-type': 'application/json', 'content-length': Buffer.byteLength(body), 'x-cdev-internal': '1' },
      }, (res) => res.resume());
      req.on('error', () => { downUntil = Date.now() + 3000; });
      if (!keepAlive) req.on('socket', (s) => s.unref && s.unref());
      req.end(body);
    } catch { /* sidecar absent: stay silent */ }
  }

  // the event loop is empty: send what's left (this keeps the process alive just for that)
  process.on('beforeExit', () => flush(true));
  // process.exit() skips beforeExit: last resort, a blocking send through a child process
  process.on('exit', () => {
    if (!queue.length || Date.now() < downUntil) return;
    try {
      const body = JSON.stringify(queue.splice(0));
      const post = `const r=require('http').request(${JSON.stringify(TARGET.origin + '/ingest')},{method:'POST',headers:{'x-cdev-internal':'1'}},s=>s.resume());r.on('error',()=>{});r.end(require('fs').readFileSync(0));`;
      require('child_process').spawnSync(process.execPath, ['-e', post], { input: body, timeout: 2000, stdio: ['pipe', 'ignore', 'ignore'], env: { PATH: process.env.PATH } });
    } catch { /* ignore */ }
  });

  // ---------- serialization (deep for watches: linked lists / trees / graphs) ----------
  function ser(root, maxDepth = 6, maxNodes = 400) {
    const seen = new Map(); // obj -> output object (for __ref)
    const onPath = new Set(); // arrays being serialized now: re-entering one is a real cycle
    let nodes = 0;
    let nextId = 1;
    function go(v, d) {
      if (v === undefined) return { __t: 'undefined' };
      if (v === null || typeof v === 'boolean') return v;
      if (typeof v === 'number') return Number.isFinite(v) ? v : { __t: 'number', v: String(v) };
      if (typeof v === 'bigint') return { __t: 'bigint', v: v.toString() };
      if (typeof v === 'string') return v.length > 2000 ? v.slice(0, 2000) + '…' : v;
      if (typeof v === 'symbol') return { __t: 'symbol', v: v.toString() };
      if (typeof v === 'function') return { __t: 'function', name: v.name || 'anonymous' };
      if (Array.isArray(v) && onPath.has(v)) return { __t: 'circular' };
      if (!Array.isArray(v) && seen.has(v)) {
        const out = seen.get(v);
        if (out && typeof out === 'object' && !Array.isArray(out)) {
          if (out.__id === undefined) out.__id = nextId++;
          return { __ref: out.__id };
        }
        return { __t: 'circular' };
      }
      if (d > maxDepth || ++nodes > maxNodes) return { __t: cname(v) || 'object', v: '…' };
      try {
        if (v instanceof Error) return { __t: v.name || 'Error', message: v.message, stack: String(v.stack || '').split('\n').slice(1, 6).map((s) => s.trim()) };
        if (v instanceof Date) return { __t: 'Date', v: isNaN(v) ? 'Invalid Date' : v.toISOString() };
        if (v instanceof RegExp) return { __t: 'RegExp', v: String(v) };
        if (typeof v.then === 'function') return { __t: 'Promise' };
        if (ArrayBuffer.isView(v) && !(v instanceof DataView)) {
          if (v.length <= 200) return Array.from(v);
          return { __t: cname(v), length: v.length };
        }
        if (v instanceof Map) {
          const out = { __t: 'Map', size: v.size, entries: [] };
          seen.set(v, out);
          for (const [k, x] of v) { if (out.entries.length >= 100) break; out.entries.push([go(k, d + 1), go(x, d + 1)]); }
          return out;
        }
        if (v instanceof Set) {
          const out = { __t: 'Set', size: v.size, values: [] };
          seen.set(v, out);
          for (const x of v) { if (out.values.length >= 100) break; out.values.push(go(x, d + 1)); }
          return out;
        }
        if (Array.isArray(v)) {
          const out = [];
          onPath.add(v);
          const n = Math.min(v.length, 500);
          for (let i = 0; i < n; i++) out.push(go(v[i], d + 1));
          if (v.length > n) out.push({ __t: 'more', n: v.length - n });
          onPath.delete(v);
          return out;
        }
        const out = {};
        seen.set(v, out);
        const c = cname(v);
        if (c && c !== 'Object') out.__class = c;
        const keys = Object.keys(v);
        // linked structures get extra depth so chains aren't cut short
        const linky = 'next' in v || 'left' in v || 'right' in v || 'children' in v;
        let i = 0;
        for (const k of keys) {
          if (i++ >= 60) { out['…'] = keys.length - 60; break; }
          let x;
          try { x = v[k]; } catch { out[k] = { __t: 'getter-error' }; continue; }
          out[k] = go(x, linky && (k === 'next' || k === 'left' || k === 'right' || k === 'children' || k === 'prev') ? d : d + 1);
        }
        return out;
      } finally { /* keep `seen` entries so DAG/shared nodes become refs */ }
    }
    return go(root, 0);
  }

  function cname(v) {
    try { const p = Object.getPrototypeOf(v); return p && p.constructor && p.constructor.name; } catch { return ''; }
  }

  const serLite = (v) => ser(v, 3, 150);

  // ---------- source locations + label inference ----------
  const SELF = __filename;
  function loc(skip = 0) {
    const stack = new Error().stack.split('\n').slice(2);
    for (const line of stack) {
      if (line.includes(SELF) || line.includes('node:') || line.includes('node_modules/')) continue;
      if (skip-- > 0) continue;
      const m = line.match(/\(?((?:file:\/\/)?[^\s()]+):(\d+):(\d+)\)?\s*$/);
      if (!m) continue;
      let file = m[1].replace(/^file:\/\//, '');
      if (file.startsWith(cwd)) file = path.relative(cwd, file);
      return { file, line: +m[2], col: +m[3], text: `${file}:${m[2]}` };
    }
    return null;
  }

  const srcCache = new Map();
  function guessLabel(l) {
    if (!l) return '';
    try {
      const full = path.isAbsolute(l.file) ? l.file : path.join(cwd, l.file);
      let lines = srcCache.get(full);
      if (!lines) { lines = fs.readFileSync(full, 'utf8').split('\n'); srcCache.set(full, lines); }
      const src = lines[l.line - 1] || '';
      const at = src.indexOf('.w(', Math.max(0, l.col - 12));
      const i = at >= 0 ? at : src.indexOf('.w(');
      if (i < 0) return '';
      // take the first argument, balancing brackets
      let depth = 0; let out = '';
      for (const ch of src.slice(i + 3)) {
        if ('([{'.includes(ch)) depth++;
        if (')]}'.includes(ch)) { if (depth === 0) break; depth--; }
        if (ch === ',' && depth === 0) break;
        out += ch;
      }
      return out.trim().slice(0, 60);
    } catch { return ''; }
  }

  // ---------- identity (JS has no addresses; stable ids show which names share an object) ----------
  const ids = new WeakMap();
  let nextObjId = 1;
  const idOf = (o) => { if (!ids.has(o)) ids.set(o, nextObjId++); return '#' + ids.get(o); };
  const isObj = (x) => x !== null && (typeof x === 'object' || typeof x === 'function');
  function mem(v) {
    if (!isObj(v)) return undefined;
    const m = { addr: idOf(v), region: 'heap' };
    try {
      if (Array.isArray(v)) m.items = v.slice(0, 64).map((x) => (isObj(x) ? idOf(x) : null));
      else if (!(v instanceof Map) && !(v instanceof Set)) {
        const f = {};
        for (const k of Object.keys(v).slice(0, 64)) if (isObj(v[k])) f[k] = idOf(v[k]);
        m.fields = f;
      }
    } catch { /* ignore */ }
    return m;
  }

  // ---------- public API ----------
  const lastWatch = new Map();
  function w(value, label) {
    try {
      const l = loc();
      const name = label || guessLabel(l) || (l ? l.text : 'value');
      const v = ser(value, 64, 2000);
      const m = mem(value);
      const key = JSON.stringify([v, m]);
      if (lastWatch.get(name) === key) return value;
      lastWatch.set(name, key);
      if (lastWatch.size > 5000) lastWatch.clear();
      send({ kind: 'watch', name, v, mem: m, loc: l && l.text, caller: als.getStore() });
    } catch { /* never break the app */ }
    return value;
  }

  function paramNames(fn) {
    try {
      const s = Function.prototype.toString.call(fn).replace(/\/\*[\s\S]*?\*\/|\/\/.*$/gm, '');
      const m = s.match(/^[^(]*\(([^)]*)\)/) || s.match(/^\s*(?:async\s+)?([A-Za-z_$][\w$]*)\s*=>/);
      if (!m) return [];
      return m[1].split(',').map((p) => p.trim().replace(/=.*$/, '').replace(/^\.\.\./, '...').trim()).filter(Boolean);
    } catch { return []; }
  }

  function trace(fn, name) {
    if (typeof fn !== 'function' || fn.__cdev) return fn;
    name = name || fn.name || 'anonymous';
    const params = paramNames(fn);
    const wrapped = function (...args) {
      const caller = als.getStore();
      const l = loc();
      const t0 = performance.now();
      const base = { kind: 'call', name, params, args: args.map(serLite), caller, loc: l && l.text };
      const done = (ret, err, isAsync) => {
        try {
          const ev = { ...base, ms: +(performance.now() - t0).toFixed(3) };
          if (err !== undefined) ev.err = String((err && err.message) || err);
          else ev.ret = serLite(ret);
          if (isAsync) ev.async = true;
          send(ev);
        } catch { /* ignore */ }
      };
      let r;
      try {
        r = als.run(name, () => (new.target ? Reflect.construct(fn, args, new.target) : fn.apply(this, args)));
      } catch (e) { done(undefined, e); throw e; }
      if (r && typeof r.then === 'function') {
        // return the derived promise so unhandled rejections still surface
        return r.then((v) => { done(v, undefined, true); return v; }, (e) => { done(undefined, e, true); throw e; });
      }
      done(r);
      return r;
    };
    try { Object.defineProperty(wrapped, 'name', { value: name }); } catch { /* ignore */ }
    try { Object.defineProperty(wrapped, 'length', { value: fn.length }); } catch { /* ignore */ }
    wrapped.__cdev = true;
    return wrapped;
  }

  function traceAll(target, prefix) {
    if (!target) return target;
    const isClass = typeof target === 'function' && target.prototype;
    const obj = isClass ? target.prototype : target;
    const pre = prefix || (isClass ? target.name : '');
    for (const k of Object.getOwnPropertyNames(obj)) {
      if (k === 'constructor') continue;
      const d = Object.getOwnPropertyDescriptor(obj, k);
      if (d && typeof d.value === 'function' && d.writable) {
        obj[k] = trace(d.value, pre ? `${pre}.${k}` : k);
      }
    }
    return target;
  }

  function log(...args) {
    send({ kind: 'log', level: 'info', text: util.format(...args), args: args.map(serLite), loc: (loc() || {}).text, caller: als.getStore() });
  }

  // ---------- auto-watch runtime (called by code rewritten with `cdev instrument`) ----------
  // __cdev_enter(fn, file) → frame; __cdev_s(frame, line, {name: () => value}) after each statement.
  let autoBudget = +(process.env.CDEV_AUTO_MAX || 20000);
  function autoEmit(ev) {
    if (autoBudget <= 0) return;
    if (--autoBudget === 0) send({ kind: 'log', level: 'warn', text: 'cdev auto-watch: event budget used up (raise CDEV_AUTO_MAX or use --only)' });
    send(ev);
  }
  const autoOnly = new Set((process.env.CDEV_ONLY || '').split(',').map((x) => x.trim()).filter(Boolean));
  globalThis.__cdev_enter = (name, file) => (autoOnly.size && !autoOnly.has(name) && !autoOnly.has(name.split('.').pop()) ? 0 : { name, file, last: new Map() });
  globalThis.__cdev_s = (f, line, getters) => {
    if (!f || autoBudget <= 0) return;
    try {
      const vals = {};
      for (const k in getters) {
        let v;
        try { v = getters[k](); } catch { continue; } // temporal dead zone / not in scope here
        if (typeof v === 'function') continue;
        vals[k] = v;
      }
      const loc = `${f.file}:${line}`;
      for (const k in vals) {
        const v = vals[k];
        if (v === null || typeof v !== 'object') continue;
        const sv = ser(v, 64, 2000);
        const key = JSON.stringify(sv);
        if (f.last.get(k) !== key) { f.last.set(k, key); autoEmit({ kind: 'watch', name: `${f.name}.${k}`, v: sv, mem: mem(v), loc, caller: f.name, auto: true }); }
      }
      const fv = {};
      for (const k in vals) fv[k] = ser(vals[k], 3, 200);
      const key = JSON.stringify(fv);
      if (f.last.get('()') !== key) { f.last.set('()', key); autoEmit({ kind: 'watch', name: `${f.name}()`, v: fv, loc, auto: true }); }
    } catch { /* never break the app */ }
  };

  const api = { w, watch: w, trace, traceAll, log, send, flush, ser };
  globalThis.__cdev = api;
  if (!('cdev' in globalThis)) globalThis.cdev = api;
  module.exports = api;

  // ---------- auto-instrumentation ----------
  // auto-hooks only when preloaded (--require / NODE_OPTIONS), not on a plain require()
  const preloaded = (process.env.NODE_OPTIONS || '').includes('cdev-node') || process.execArgv.some((a) => a.includes('cdev-node'));
  const autoOn = process.env.CDEV_AUTO === '1' || preloaded;
  if (!autoOn) return;

  // auto-watch: rewrite project files as they load (`cdev --auto run|watch`)
  if (process.env.CDEV_AUTO_WATCH) installAutoHooks();

  // console.*
  for (const m of ['log', 'info', 'warn', 'error', 'debug']) {
    const orig = console[m];
    if (typeof orig !== 'function') continue;
    console[m] = function (...a) {
      try {
        send({ kind: 'log', level: m === 'log' ? 'info' : m, text: util.format(...a), args: a.map(serLite), loc: (loc() || {}).text, caller: als.getStore() });
      } catch { /* ignore */ }
      return orig.apply(this, a);
    };
  }

  // uncaught errors (monitor only: does not change crash behaviour)
  process.on('uncaughtExceptionMonitor', (err, origin) => {
    try {
      send({ kind: 'error', msg: String((err && err.message) || err), stack: String((err && err.stack) || ''), origin });
      flush();
    } catch { /* ignore */ }
  });

  function headersOf(h) {
    const out = {};
    for (const [k, v] of Object.entries(h || {})) {
      const s = Array.isArray(v) ? v.join(', ') : String(v);
      out[k] = REDACT.test(k) ? s.slice(0, 10) + '…' : s;
    }
    return out;
  }

  function bodyText(chunks, headers) {
    if (!chunks.length) return undefined;
    const ct = String((headers && (headers['content-type'] || headers['Content-Type'])) || '');
    const enc = headers && (headers['content-encoding'] || headers['Content-Encoding']);
    const buf = Buffer.concat(chunks.map((c) => (typeof c === 'string' ? Buffer.from(c) : Buffer.from(c))));
    if (enc && enc !== 'identity') return `<${enc} ${buf.length} bytes>`;
    if (ct && !/json|text|xml|form|javascript|graphql/i.test(ct)) return `<${ct.split(';')[0]} ${buf.length} bytes>`;
    const s = buf.toString('utf8', 0, MAX_BODY);
    return buf.length > MAX_BODY ? s + `… (${buf.length} bytes)` : s;
  }

  function collect(chunks, c, state) {
    if (c == null || state.n > MAX_BODY) return;
    const b = typeof c === 'string' ? Buffer.from(c) : c;
    if (!Buffer.isBuffer(b) && !(b instanceof Uint8Array)) return;
    chunks.push(b);
    state.n += b.length;
  }

  // correlation id: lets the sidecar pair the client's and the server's record of one call
  const newCid = () => Math.random().toString(36).slice(2, 10) + Date.now().toString(36).slice(-4);
  const isLocal = (h) => /^(localhost|127\.0\.0\.1|\[::1\]|0\.0\.0\.0)(:|$)/.test(h || '');

  // outgoing http/https
  function patchClient(mod, proto) {
    const orig = mod.request;
    mod.request = function (...args) {
      const req = orig.apply(this, args);
      try { watchClient(req, proto); } catch { /* ignore */ }
      return req;
    };
    mod.get = function (...args) {
      const req = mod.request(...args);
      req.end();
      return req;
    };
  }

  function watchClient(req, proto) {
    if (req.getHeader('x-cdev-internal')) return;
    const t0 = performance.now();
    const host = req.getHeader('host') || req.host;
    const url = `${proto}//${host}${req.path}`;
    const caller = als.getStore();
    let cid;
    if (isLocal(host)) { try { cid = newCid(); req.setHeader('x-cdev-from', SVC); req.setHeader('x-cdev-id', cid); } catch { cid = undefined; } }
    const reqChunks = []; const rs = { n: 0 };
    const ow = req.write; const oe = req.end;
    req.write = function (c, ...r) { collect(reqChunks, c, rs); return ow.call(this, c, ...r); };
    req.end = function (c, ...r) { if (typeof c !== 'function') collect(reqChunks, c, rs); return oe.call(this, c, ...r); };
    let sent = false;
    const finish = (res, err) => {
      if (sent) return; sent = true;
      const reqHeaders = headersOf(req.getHeaders());
      send({
        kind: 'http', dir: 'out', method: req.method, url, caller, cid,
        status: res && res.statusCode, ms: +(performance.now() - t0).toFixed(2), err: err ? String(err.message || err) : undefined,
        req: { headers: reqHeaders, body: bodyText(reqChunks, reqHeaders) },
        res: res ? { headers: headersOf(res.headers), body: bodyText(res.__cdevChunks || [], res.headers) } : undefined,
      });
    };
    req.on('response', (res) => {
      const chunks = []; const st = { n: 0 };
      res.__cdevChunks = chunks;
      const emit = res.emit;
      res.emit = function (ev, c, ...r) { if (ev === 'data') collect(chunks, c, st); return emit.call(this, ev, c, ...r); };
      res.on('end', () => finish(res));
      res.on('close', () => finish(res));
      if (req.listenerCount('response') === 1) res.resume(); // nobody else listening: drain like node would
    });
    req.on('error', (e) => finish(null, e));
  }

  patchClient(http, 'http:');
  patchClient(https, 'https:');

  // outgoing fetch (undici)
  if (typeof globalThis.fetch === 'function') {
    const of = globalThis.fetch;
    globalThis.fetch = async function (input, init) {
      const t0 = performance.now();
      const caller = als.getStore();
      let url; let method; let cid;
      try {
        url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        method = ((init && init.method) || (input && input.method) || 'GET').toUpperCase();
        const u = new URL(url);
        if (isLocal(u.host)) {
          const h = new Headers((init && init.headers) || (input && input.headers) || undefined);
          cid = newCid();
          h.set('x-cdev-from', SVC);
          h.set('x-cdev-id', cid);
          init = { ...(init || {}), headers: h };
        }
      } catch { /* leave request untouched */ }
      const reqHeaders = {};
      try { new Headers((init && init.headers) || (input && input.headers) || undefined).forEach((v, k) => { reqHeaders[k] = REDACT.test(k) ? v.slice(0, 10) + '…' : v; }); } catch { /* ignore */ }
      const reqBody = init && typeof init.body === 'string' ? init.body.slice(0, MAX_BODY) : init && init.body ? `<${cname(init.body) || 'body'}>` : undefined;
      let res;
      try {
        res = await of.call(this, input, init);
      } catch (e) {
        send({ kind: 'http', dir: 'out', via: 'fetch', method, url, caller, cid, ms: +(performance.now() - t0).toFixed(2), err: String(e.message || e), req: { headers: reqHeaders, body: reqBody } });
        throw e;
      }
      try {
        const resHeaders = {};
        res.headers.forEach((v, k) => { resHeaders[k] = REDACT.test(k) ? v.slice(0, 10) + '…' : v; });
        const ct = res.headers.get('content-type') || '';
        const ev = { kind: 'http', dir: 'out', via: 'fetch', method, url, caller, cid, status: res.status, ms: +(performance.now() - t0).toFixed(2), req: { headers: reqHeaders, body: reqBody }, res: { headers: resHeaders } };
        if (/json|text|xml|javascript|graphql/i.test(ct) && !/event-stream/.test(ct)) {
          res.clone().text().then((t) => { ev.res.body = t.length > MAX_BODY ? t.slice(0, MAX_BODY) + '…' : t; send(ev); }, () => send(ev));
        } else {
          send(ev);
        }
      } catch { /* ignore */ }
      return res;
    };
  }

  // incoming http(s) servers + port registration
  function patchServer(Server) {
    if (!Server || !Server.prototype) return;
    const origEmit = Server.prototype.emit;
    Server.prototype.emit = function (ev, req, res) {
      if (ev === 'listening') {
        try { const a = this.address(); if (a && a.port) send({ kind: 'service', name: SVC, port: a.port }); } catch { /* ignore */ }
      }
      if (ev === 'request' && req && res && !IGNORE.test(req.url || '')) {
        let route = `${req.method} ${(req.url || '/').split('?')[0]}`;
        try { watchIncoming(req, res, route); } catch { /* ignore */ }
        return als.run(route, () => origEmit.apply(this, arguments));
      }
      return origEmit.apply(this, arguments);
    };
  }

  function watchIncoming(req, res, route) {
    const t0 = performance.now();
    const reqChunks = []; const rs = { n: 0 };
    const remit = req.emit;
    req.emit = function (ev, c, ...r) { if (ev === 'data') collect(reqChunks, c, rs); return remit.call(this, ev, c, ...r); };
    const resChunks = []; const ss = { n: 0 };
    const ow = res.write; const oe = res.end;
    res.write = function (c, ...r) { if (typeof c !== 'function') collect(resChunks, c, ss); return ow.call(this, c, ...r); };
    res.end = function (c, ...r) { if (typeof c !== 'function') collect(resChunks, c, ss); return oe.call(this, c, ...r); };
    let sent = false;
    const finish = () => {
      if (sent) return; sent = true;
      const ua = String(req.headers['user-agent'] || '');
      const from = req.headers['x-cdev-from'] || (/Mozilla/.test(ua) ? 'browser' : 'client');
      const resHeaders = headersOf(res.getHeaders());
      send({
        kind: 'http', dir: 'in', method: req.method, url: `http://${req.headers.host || 'localhost'}${req.url}`, from,
        cid: req.headers['x-cdev-id'], handler: route,
        status: res.statusCode, ms: +(performance.now() - t0).toFixed(2),
        req: { headers: headersOf(req.headers), body: bodyText(reqChunks, req.headers) },
        res: { headers: resHeaders, body: bodyText(resChunks, resHeaders) },
      });
    };
    res.on('finish', finish);
    res.on('close', finish);
  }

  patchServer(http.Server);
  patchServer(https.Server);

  function installAutoHooks() {
    const exts = /\.(m?[jt]sx?|c[jt]s)$/;
    const wanted = (file) => file.startsWith(cwd + path.sep) && !file.includes(`${path.sep}node_modules${path.sep}`) && file !== __filename && exts.test(file);
    const relName = (file) => path.relative(cwd, file);

    // CommonJS (plain node, tsx's CJS mode): synchronous rewrite via the cdev binary
    const Module = require('module');
    const bin = process.env.CDEV_BIN;
    if (bin) {
      const cp = require('child_process');
      const orig = Module.prototype._compile;
      Module.prototype._compile = function (content, filename) {
        if (wanted(filename)) {
          try { content = cp.execFileSync(bin, ['instrument', '--name', relName(filename)], { input: content, encoding: 'utf8', maxBuffer: 64 << 20, stdio: ['pipe', 'pipe', 'ignore'] }); } catch { /* run unmodified */ }
        }
        return orig.call(this, content, filename);
      };
    }

    // ES modules: a loader hook asks the sidecar for the rewrite
    if (typeof Module.register === 'function' && typeof Bun === 'undefined') {
      const loader = `
        const root = ${JSON.stringify(require('url').pathToFileURL(cwd).href + '/')};
        const api = ${JSON.stringify(TARGET.origin + '/api/instrument?file=')};
        export async function load(url, context, next) {
          const r = await next(url, context);
          if (!url.startsWith(root) || url.includes('/node_modules/') || r.format !== 'module' || r.source == null) return r;
          try {
            const src = typeof r.source === 'string' ? r.source : new TextDecoder().decode(r.source);
            const res = await fetch(api + encodeURIComponent(decodeURIComponent(url.slice(root.length))), { method: 'POST', body: src });
            if (res.ok) return { ...r, source: await res.text(), shortCircuit: true };
          } catch {}
          return r;
        }`;
      try { Module.register('data:text/javascript,' + encodeURIComponent(loader)); } catch { /* older node */ }
    }

    // Bun (cdev watch app.ts): runtime plugin
    if (typeof Bun !== 'undefined' && Bun.plugin) {
      const esc = cwd.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      Bun.plugin({
        name: 'cdev-auto',
        setup(build) {
          build.onLoad({ filter: new RegExp('^' + esc + '/(?!node_modules/).*\\.(m?[jt]sx?|c[jt]s)$') }, async (args) => {
            const src = await Bun.file(args.path).text();
            const ext = args.path.split('.').pop();
            const loader = { mts: 'ts', cts: 'ts', mjs: 'js', cjs: 'js' }[ext] || ext;
            try {
              const res = await fetch(`${TARGET.origin}/api/instrument?file=${encodeURIComponent(relName(args.path))}`, { method: 'POST', body: src });
              return { contents: res.ok ? await res.text() : src, loader };
            } catch { return { contents: src, loader }; }
          });
        },
      });
    }
  }
})();
