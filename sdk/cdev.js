/* cdev browser SDK — <script src="http://localhost:4400/cdev.js"></script>
 *
 * window.cdev.w(value, label?)  record a value + inferred type, returns it
 * window.cdev.trace(fn, name?)  record calls: args, return, errors, duration
 * window.cdev.log(...args)
 *
 * Auto: console.*, window errors, unhandled rejections, fetch, XMLHttpRequest.
 */
(function () {
  'use strict';
  if (window.cdev && window.cdev.__v) return;

  var script = document.currentScript;
  var ORIGIN = script && script.src ? new URL(script.src).origin : 'http://127.0.0.1:4400';
  var INGEST = ORIGIN + '/ingest';
  var SVC = (script && script.dataset && script.dataset.service) || 'browser';
  var MAX_BODY = 16384;
  var REDACT = /^(authorization|cookie|x-api-key)$/i;
  var origFetch = window.fetch && window.fetch.bind(window);

  // ---------- transport ----------
  var queue = [];
  var timer = null;
  function send(ev) {
    ev.ts = ev.ts || Date.now();
    ev.svc = ev.svc || SVC;
    ev.lang = 'browser';
    queue.push(ev);
    if (queue.length > 3000) queue.shift();
    if (!timer) timer = setTimeout(flush, 60);
  }
  function flush() {
    timer = null;
    if (!queue.length) return;
    var body;
    try { body = JSON.stringify(queue); } catch (e) { queue = []; return; }
    queue = [];
    try {
      // text/plain + no-cors: no preflight, works from any origin
      origFetch(INGEST, { method: 'POST', body: body, mode: 'no-cors', keepalive: body.length < 60000, headers: { 'content-type': 'text/plain' } }).catch(function () {});
    } catch (e) { /* ignore */ }
  }
  window.addEventListener('pagehide', function () {
    if (queue.length && navigator.sendBeacon) { navigator.sendBeacon(INGEST, JSON.stringify(queue)); queue = []; }
  });

  // ---------- serialization ----------
  function cname(v) { try { var p = Object.getPrototypeOf(v); return p && p.constructor && p.constructor.name; } catch (e) { return ''; } }
  function ser(root, maxDepth, maxNodes) {
    maxDepth = maxDepth || 6; maxNodes = maxNodes || 400;
    var seen = new Map(); var onPath = new Set(); var nodes = 0; var nextId = 1;
    function go(v, d) {
      if (v === undefined) return { __t: 'undefined' };
      if (v === null || typeof v === 'boolean') return v;
      if (typeof v === 'number') return isFinite(v) ? v : { __t: 'number', v: String(v) };
      if (typeof v === 'bigint') return { __t: 'bigint', v: v.toString() };
      if (typeof v === 'string') return v.length > 2000 ? v.slice(0, 2000) + '…' : v;
      if (typeof v === 'symbol') return { __t: 'symbol', v: v.toString() };
      if (typeof v === 'function') return { __t: 'function', name: v.name || 'anonymous' };
      if (Array.isArray(v) && onPath.has(v)) return { __t: 'circular' };
      if (!Array.isArray(v) && seen.has(v)) {
        var o = seen.get(v);
        if (o && typeof o === 'object' && !Array.isArray(o)) { if (o.__id === undefined) o.__id = nextId++; return { __ref: o.__id }; }
        return { __t: 'circular' };
      }
      if (d > maxDepth || ++nodes > maxNodes) return { __t: cname(v) || 'object', v: '…' };
      if (v instanceof Error) return { __t: v.name || 'Error', message: v.message, stack: String(v.stack || '').split('\n').slice(1, 6) };
      if (v instanceof Date) return { __t: 'Date', v: isNaN(v) ? 'Invalid Date' : v.toISOString() };
      if (typeof Element !== 'undefined' && v instanceof Element) return { __t: 'Element', v: '<' + v.tagName.toLowerCase() + (v.id ? '#' + v.id : '') + (v.className && typeof v.className === 'string' ? '.' + v.className.split(' ').join('.') : '') + '>' };
      if (typeof Event !== 'undefined' && v instanceof Event) return { __t: cname(v), type: v.type };
      if (v === window || v === document) return { __t: v === window ? 'Window' : 'Document' };
      if (typeof v.then === 'function') return { __t: 'Promise' };
      if (v.$$typeof) return { __t: 'ReactElement', type: typeof v.type === 'string' ? v.type : (v.type && (v.type.displayName || v.type.name)) || '?' };
      if (ArrayBuffer.isView(v) && !(v instanceof DataView)) return v.length <= 200 ? Array.from(v) : { __t: cname(v), length: v.length };
      if (v instanceof Map) {
        var m = { __t: 'Map', size: v.size, entries: [] }; seen.set(v, m);
        v.forEach(function (x, k) { if (m.entries.length < 100) m.entries.push([go(k, d + 1), go(x, d + 1)]); });
        return m;
      }
      if (v instanceof Set) {
        var s = { __t: 'Set', size: v.size, values: [] }; seen.set(v, s);
        v.forEach(function (x) { if (s.values.length < 100) s.values.push(go(x, d + 1)); });
        return s;
      }
      if (Array.isArray(v)) {
        var a = []; onPath.add(v);
        var n = Math.min(v.length, 500);
        for (var i = 0; i < n; i++) a.push(go(v[i], d + 1));
        if (v.length > n) a.push({ __t: 'more', n: v.length - n });
        onPath.delete(v);
        return a;
      }
      var out = {}; seen.set(v, out);
      var c = cname(v); if (c && c !== 'Object') out.__class = c;
      var keys = Object.keys(v);
      var linky = 'next' in v || 'left' in v || 'right' in v || 'children' in v;
      for (var j = 0; j < keys.length && j < 60; j++) {
        var k = keys[j]; var x;
        try { x = v[k]; } catch (e) { out[k] = { __t: 'getter-error' }; continue; }
        var structural = k === 'next' || k === 'prev' || k === 'left' || k === 'right' || k === 'children';
        out[k] = go(x, linky && structural ? d : d + 1);
      }
      return out;
    }
    return go(root, 0);
  }
  function serLite(v) { return ser(v, 3, 150); }

  // ---------- locations ----------
  function loc() {
    var lines = String(new Error().stack || '').split('\n').slice(2);
    for (var i = 0; i < lines.length; i++) {
      var l = lines[i];
      if (l.indexOf('/cdev.js') >= 0) continue;
      var m = l.match(/(https?:\/\/[^\s)]+?):(\d+):(\d+)/);
      if (!m) continue;
      var file = m[1].replace(location.origin + '/', '').replace(/\?.*$/, '');
      return file + ':' + m[2];
    }
    return undefined;
  }

  // ---------- identity (no addresses in JS; stable ids show shared objects) ----------
  var ids = new WeakMap(); var nextObjId = 1;
  function idOf(o) { if (!ids.has(o)) ids.set(o, nextObjId++); return '#' + ids.get(o); }
  function isObj(x) { return x !== null && (typeof x === 'object' || typeof x === 'function'); }
  function mem(v) {
    if (!isObj(v)) return undefined;
    var m = { addr: idOf(v), region: 'heap' };
    try {
      if (Array.isArray(v)) m.items = v.slice(0, 64).map(function (x) { return isObj(x) ? idOf(x) : null; });
      else if (!(v instanceof Map) && !(v instanceof Set)) {
        var f = {}; Object.keys(v).slice(0, 64).forEach(function (k) { if (isObj(v[k])) f[k] = idOf(v[k]); }); m.fields = f;
      }
    } catch (e) { /* ignore */ }
    return m;
  }

  // ---------- API ----------
  var lastWatch = new Map();
  function w(value, label) {
    try {
      var l = loc();
      var name = label || l || 'value';
      var v = ser(value, 64, 2000);
      var m = mem(value);
      var key = JSON.stringify([v, m]);
      if (lastWatch.get(name) === key) return value;
      lastWatch.set(name, key);
      send({ kind: 'watch', name: name, v: v, mem: m, loc: l, caller: stack[stack.length - 1] });
    } catch (e) { /* ignore */ }
    return value;
  }

  var stack = [];
  function paramNames(fn) {
    try {
      var s = Function.prototype.toString.call(fn);
      var m = s.match(/^[^(]*\(([^)]*)\)/) || s.match(/^\s*(?:async\s+)?([A-Za-z_$][\w$]*)\s*=>/);
      return m ? m[1].split(',').map(function (p) { return p.trim().replace(/=.*$/, '').trim(); }).filter(Boolean) : [];
    } catch (e) { return []; }
  }
  function trace(fn, name) {
    if (typeof fn !== 'function' || fn.__cdev) return fn;
    name = name || fn.name || 'anonymous';
    var params = paramNames(fn);
    var wrapped = function () {
      var args = Array.prototype.slice.call(arguments);
      var t0 = performance.now();
      var base = { kind: 'call', name: name, params: params, args: args.map(serLite), caller: stack[stack.length - 1], loc: loc() };
      function done(ret, err, isAsync) {
        var ev = Object.assign({}, base, { ms: +(performance.now() - t0).toFixed(3) });
        if (err !== undefined) ev.err = String((err && err.message) || err); else ev.ret = serLite(ret);
        if (isAsync) ev.async = true;
        send(ev);
      }
      var r;
      stack.push(name);
      try { r = new.target ? Reflect.construct(fn, args, new.target) : fn.apply(this, args); }
      catch (e) { stack.pop(); done(undefined, e); throw e; }
      stack.pop();
      if (r && typeof r.then === 'function') {
        return r.then(function (v) { done(v, undefined, true); return v; }, function (e) { done(undefined, e, true); throw e; });
      }
      done(r);
      return r;
    };
    try { Object.defineProperty(wrapped, 'name', { value: name }); } catch (e) { /* ignore */ }
    wrapped.__cdev = true;
    return wrapped;
  }
  function traceAll(target, prefix) {
    var isClass = typeof target === 'function' && target.prototype;
    var obj = isClass ? target.prototype : target;
    var pre = prefix || (isClass ? target.name : '');
    Object.getOwnPropertyNames(obj).forEach(function (k) {
      if (k === 'constructor') return;
      var d = Object.getOwnPropertyDescriptor(obj, k);
      if (d && typeof d.value === 'function' && d.writable) obj[k] = trace(d.value, pre ? pre + '.' + k : k);
    });
    return target;
  }
  function fmt(args) {
    return args.map(function (a) {
      if (typeof a === 'string') return a;
      try { return JSON.stringify(ser(a, 3, 100)); } catch (e) { return String(a); }
    }).join(' ');
  }
  function log() {
    var a = Array.prototype.slice.call(arguments);
    send({ kind: 'log', level: 'info', text: fmt(a), args: a.map(serLite), loc: loc() });
  }

  // ---------- auto-watch runtime (for pages served by `cdev --auto watch`) ----------
  var autoBudget = 20000;
  function autoEmit(ev) {
    if (autoBudget <= 0) return;
    if (--autoBudget === 0) send({ kind: 'log', level: 'warn', text: 'cdev auto-watch: event budget used up' });
    send(ev);
  }
  window.__cdev_enter = function (name, file) { return { name: name, file: file, last: new Map() }; };
  window.__cdev_s = function (f, line, getters) {
    if (!f || autoBudget <= 0) return;
    try {
      var vals = {}; var k;
      for (k in getters) {
        var v;
        try { v = getters[k](); } catch (e) { continue; }
        if (typeof v === 'function') continue;
        vals[k] = v;
      }
      var loc = f.file + ':' + line;
      for (k in vals) {
        var x = vals[k];
        if (x === null || typeof x !== 'object') continue;
        var sv = ser(x, 64, 2000); var key = JSON.stringify(sv);
        if (f.last.get(k) !== key) { f.last.set(k, key); autoEmit({ kind: 'watch', name: f.name + '.' + k, v: sv, mem: mem(x), loc: loc, caller: f.name, auto: true }); }
      }
      var fv = {};
      for (k in vals) fv[k] = ser(vals[k], 3, 200);
      var fkey = JSON.stringify(fv);
      if (f.last.get('()') !== fkey) { f.last.set('()', fkey); autoEmit({ kind: 'watch', name: f.name + '()', v: fv, loc: loc, auto: true }); }
    } catch (e) { /* never break the page */ }
  };

  window.cdev = { __v: 1, w: w, watch: w, trace: trace, traceAll: traceAll, log: log, send: send, flush: flush };

  // ---------- auto ----------
  ['log', 'info', 'warn', 'error', 'debug'].forEach(function (m) {
    var orig = console[m];
    if (typeof orig !== 'function') return;
    console[m] = function () {
      var a = Array.prototype.slice.call(arguments);
      try { send({ kind: 'log', level: m === 'log' ? 'info' : m, text: fmt(a), args: a.map(serLite), loc: loc() }); } catch (e) { /* ignore */ }
      return orig.apply(this, a);
    };
  });

  window.addEventListener('error', function (e) {
    send({ kind: 'error', msg: e.message, stack: e.error && e.error.stack, loc: e.filename ? e.filename.replace(location.origin + '/', '') + ':' + e.lineno : undefined });
  });
  window.addEventListener('unhandledrejection', function (e) {
    var r = e.reason;
    send({ kind: 'error', msg: 'Unhandled rejection: ' + ((r && r.message) || String(r)), stack: r && r.stack });
  });

  function hdrs(h) {
    var out = {};
    try { new Headers(h || undefined).forEach(function (v, k) { out[k] = REDACT.test(k) ? v.slice(0, 10) + '…' : v; }); } catch (e) { /* ignore */ }
    return out;
  }
  function bodyOf(b) {
    if (b == null) return undefined;
    if (typeof b === 'string') return b.slice(0, MAX_BODY);
    if (b instanceof URLSearchParams) return b.toString();
    if (typeof FormData !== 'undefined' && b instanceof FormData) { var o = {}; b.forEach(function (v, k) { o[k] = typeof v === 'string' ? v : '<file>'; }); return JSON.stringify(o); }
    return '<' + (cname(b) || 'body') + '>';
  }
  function absUrl(u) { try { return new URL(u, location.href).href; } catch (e) { return String(u); } }

  if (origFetch) {
    window.fetch = function (input, init) {
      var t0 = performance.now();
      var url = absUrl(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url);
      if (url.indexOf(INGEST) === 0) return origFetch(input, init);
      var method = ((init && init.method) || (input && input.method) || 'GET').toUpperCase();
      var cid;
      try {
        // custom headers only same-origin (cross-origin would trigger CORS preflights); the sidecar pairs the rest by timing
        if (new URL(url).origin === location.origin) {
          var h = new Headers((init && init.headers) || (input && input.headers) || undefined);
          cid = Math.random().toString(36).slice(2, 12);
          h.set('x-cdev-from', SVC);
          h.set('x-cdev-id', cid);
          init = Object.assign({}, init || {}, { headers: h });
        }
      } catch (e) { /* ignore */ }
      var reqInfo = { headers: hdrs((init && init.headers) || (input && input.headers)), body: bodyOf(init && init.body) };
      var caller = stack[stack.length - 1];
      return origFetch(input, init).then(function (res) {
        var ev = { kind: 'http', dir: 'out', via: 'fetch', method: method, url: url, caller: caller, cid: cid, status: res.status, ms: +(performance.now() - t0).toFixed(2), req: reqInfo, res: { headers: hdrs(res.headers) } };
        var ct = res.headers.get('content-type') || '';
        if (/json|text|xml|graphql/i.test(ct) && !/event-stream/.test(ct)) {
          res.clone().text().then(function (t) { ev.res.body = t.length > MAX_BODY ? t.slice(0, MAX_BODY) + '…' : t; send(ev); }, function () { send(ev); });
        } else send(ev);
        return res;
      }, function (err) {
        send({ kind: 'http', dir: 'out', via: 'fetch', method: method, url: url, caller: caller, ms: +(performance.now() - t0).toFixed(2), err: String((err && err.message) || err), req: reqInfo });
        throw err;
      });
    };
  }

  if (window.XMLHttpRequest) {
    var XO = XMLHttpRequest.prototype.open;
    var XS = XMLHttpRequest.prototype.send;
    var XH = XMLHttpRequest.prototype.setRequestHeader;
    XMLHttpRequest.prototype.open = function (m, u) { this.__cdev = { method: String(m).toUpperCase(), url: absUrl(u), headers: {} }; return XO.apply(this, arguments); };
    XMLHttpRequest.prototype.setRequestHeader = function (k, v) { if (this.__cdev) this.__cdev.headers[k] = REDACT.test(k) ? String(v).slice(0, 10) + '…' : v; return XH.apply(this, arguments); };
    XMLHttpRequest.prototype.send = function (body) {
      var x = this; var info = x.__cdev;
      if (info && info.url.indexOf(INGEST) !== 0) {
        var t0 = performance.now();
        x.addEventListener('loadend', function () {
          var resHeaders = {};
          String(x.getAllResponseHeaders() || '').trim().split(/[\r\n]+/).forEach(function (l) { var i = l.indexOf(':'); if (i > 0) resHeaders[l.slice(0, i).trim().toLowerCase()] = l.slice(i + 1).trim(); });
          var rb;
          try { rb = (x.responseType === '' || x.responseType === 'text') ? String(x.responseText).slice(0, MAX_BODY) : '<' + x.responseType + '>'; } catch (e) { rb = undefined; }
          send({ kind: 'http', dir: 'out', via: 'xhr', method: info.method, url: info.url, status: x.status || undefined, err: x.status ? undefined : 'network error', ms: +(performance.now() - t0).toFixed(2), req: { headers: info.headers, body: bodyOf(body) }, res: { headers: resHeaders, body: rb } });
        });
      }
      return XS.apply(this, arguments);
    };
  }
})();
