"""cdev Python SDK — stdlib only.

    import cdev
    x = cdev.w(expr)              # record value + inferred type, returns it
    @cdev.trace                   # record args (by name), return value, errors, duration, caller
    def f(a, b): ...
    app = cdev.asgi(app)          # FastAPI / Starlette incoming requests
    app = cdev.wsgi(app)          # Flask / Django incoming requests

Under `cdev run`, a sitecustomize hook calls `cdev._auto()`: logging, uncaught
exceptions and outgoing http.client/urllib requests are captured automatically.
Disable with CDEV_OFF=1.
"""

import atexit
import contextvars
import dataclasses
import functools
import http.client
import inspect
import json
import linecache
import logging
import os
import sys
import threading
import time
import traceback
from urllib.parse import urlparse

URL = urlparse(os.environ.get("CDEV_URL", "http://127.0.0.1:4400"))
SVC = os.environ.get("CDEV_SERVICE") or os.path.basename(os.getcwd())
OFF = bool(os.environ.get("CDEV_OFF"))
MAX_BODY = 16 * 1024
_CWD = os.getcwd()
_SELF = os.path.abspath(__file__)
_caller = contextvars.ContextVar("cdev_caller", default=None)
_RawConn = http.client.HTTPConnection  # class object; our requests are marked with a header

# ---------------------------------------------------------------- transport
_q = []
_lock = threading.Lock()
_wake = threading.Event()
_down_until = 0.0


def send(ev):
    if OFF:
        return
    ev.setdefault("ts", time.time() * 1000)
    ev.setdefault("svc", SVC)
    ev["lang"] = "python"
    ev["pid"] = os.getpid()
    with _lock:
        _q.append(ev)
        if len(_q) > 5000:
            del _q[0]
    _wake.set()


def flush():
    global _down_until
    with _lock:
        batch = _q[:]
        _q.clear()
    if not batch or time.time() < _down_until:
        return
    try:
        body = json.dumps(batch, default=str)
        c = _RawConn(URL.hostname, URL.port or 80, timeout=2)
        c.request("POST", "/ingest", body=body, headers={"content-type": "application/json", "x-cdev-internal": "1"})
        c.getresponse().read()
        c.close()
    except Exception:
        _down_until = time.time() + 3


def _worker():
    while True:
        _wake.wait(0.2)
        _wake.clear()
        time.sleep(0.03)
        flush()


if not OFF:
    threading.Thread(target=_worker, name="cdev", daemon=True).start()
    atexit.register(flush)

# ---------------------------------------------------------------- serialization


def ser(root, max_depth=6, max_nodes=400):
    """JSON-safe snapshot. Shared/cyclic objects become {"__ref": id}."""
    seen = {}
    on_path = set()  # lists being serialized right now: re-entering one is a real cycle
    ids = [0]
    nodes = [0]
    structural = {"next", "prev", "left", "right", "children"}

    def go(v, d):
        if v is None or isinstance(v, (bool, int, float)):
            if isinstance(v, float) and (v != v or v in (float("inf"), float("-inf"))):
                return {"__t": "float", "v": str(v)}
            return v
        if isinstance(v, str):
            return v if len(v) <= 2000 else v[:2000] + "…"
        if isinstance(v, bytes):
            return {"__t": "bytes", "v": repr(v[:80])}
        oid = id(v)
        if isinstance(v, (list, tuple)) or type(v).__name__ in ("deque", "array"):
            if oid in on_path:
                return {"__t": "circular"}
        elif oid in seen:
            out = seen[oid]
            if isinstance(out, dict):
                if "__id" not in out:
                    ids[0] += 1
                    out["__id"] = ids[0]
                return {"__ref": out["__id"]}
            return {"__t": "circular"}
        nodes[0] += 1
        if d > max_depth or nodes[0] > max_nodes:
            return {"__t": type(v).__name__, "v": "…"}
        if isinstance(v, BaseException):
            return {"__t": type(v).__name__, "message": str(v)}
        if callable(v) and not hasattr(v, "__dict__") or inspect.isfunction(v) or inspect.ismethod(v) or inspect.isclass(v):
            return {"__t": "function", "name": getattr(v, "__qualname__", repr(v))}
        if hasattr(v, "isoformat"):
            return {"__t": type(v).__name__, "v": v.isoformat()}
        if isinstance(v, dict):
            out = {}
            seen[oid] = out
            if type(v) is not dict:
                out["__class"] = type(v).__name__
            for i, (k, x) in enumerate(v.items()):
                if i >= 60:
                    out["…"] = len(v) - 60
                    break
                out[str(k)] = go(x, d + 1)
            return out
        if isinstance(v, (set, frozenset)):
            out = {"__t": "Set", "size": len(v), "values": []}
            seen[oid] = out
            out["values"] = [go(x, d + 1) for x in list(v)[:100]]
            return out
        if isinstance(v, (list, tuple)) or type(v).__name__ in ("deque", "array"):
            out = []
            on_path.add(oid)
            items = list(v)
            for x in items[:500]:
                out.append(go(x, d + 1))
            if len(items) > 500:
                out.append({"__t": "more", "n": len(items) - 500})
            on_path.discard(oid)
            return out
        # numpy / pandas without importing them
        if hasattr(v, "tolist") and hasattr(v, "shape"):
            try:
                return go(v.tolist(), d)
            except Exception:
                pass
        # dataclasses, pydantic, namedtuple-ish, plain objects
        fields = None
        if dataclasses.is_dataclass(v):
            fields = {f.name: getattr(v, f.name, None) for f in dataclasses.fields(v)}
        elif hasattr(v, "model_dump"):
            try:
                fields = v.model_dump()
            except Exception:
                fields = None
        elif hasattr(v, "__dict__"):
            fields = vars(v)
        elif hasattr(v, "__slots__"):
            fields = {s: getattr(v, s, None) for s in v.__slots__ if hasattr(v, s)}
        if fields is not None:
            out = {"__class": type(v).__name__}
            seen[oid] = out
            linky = any(k in fields for k in structural)
            for i, (k, x) in enumerate(fields.items()):
                if i >= 60:
                    break
                if k.startswith("_"):
                    continue
                out[k] = go(x, d if linky and k in structural else d + 1)
            return out
        return {"__t": type(v).__name__, "v": repr(v)[:200]}

    return go(root, 0)


def _lite(v):
    return ser(v, 3, 150)


# ---------------------------------------------------------------- locations


def _loc(frame):
    while frame and os.path.abspath(frame.f_code.co_filename) == _SELF:
        frame = frame.f_back
    if not frame:
        return None, None
    f = frame.f_code.co_filename
    rel = os.path.relpath(f, _CWD) if f.startswith(_CWD) else f
    return f"{rel}:{frame.f_lineno}", frame


def _guess_label(frame):
    try:
        src = linecache.getline(frame.f_code.co_filename, frame.f_lineno)
        i = src.find(".w(")
        if i < 0:
            return ""
        depth, out = 0, ""
        for ch in src[i + 3:]:
            if ch in "([{":
                depth += 1
            if ch in ")]}":
                if depth == 0:
                    break
                depth -= 1
            if ch == "," and depth == 0:
                break
            out += ch
        return out.strip()[:60]
    except Exception:
        return ""


# ---------------------------------------------------------------- API

_last = {}


def _mem(v):
    """CPython object identity: id() is the object's address. Element ids expose
    aliasing, e.g. [[0] * 3] * 3 is three references to one inner list."""
    m = {"addr": hex(id(v)), "size": sys.getsizeof(v), "region": "heap"}
    # only link containers/objects: numbers and strings are objects too, but
    # showing each one would bury the interesting references
    ref = lambda x: None if isinstance(x, (int, float, str, bytes, bool, type(None))) else hex(id(x))
    try:
        if isinstance(v, (list, tuple)):
            m["items"] = [ref(x) for x in v[:64]]
        elif isinstance(v, dict):
            m["fields"] = {str(k): r for k, x in list(v.items())[:64] if (r := ref(x))}
        elif hasattr(v, "__dict__") and not inspect.isclass(v) and not inspect.ismodule(v):
            m["fields"] = {k: r for k, x in list(vars(v).items())[:64] if not k.startswith("_") and (r := ref(x))}
    except Exception:
        pass
    return m


def w(value, label=None):
    """Record `value` (with inferred type shape) and return it unchanged."""
    if OFF:
        return value
    try:
        loc, frame = _loc(sys._getframe(1))
        name = label or (_guess_label(frame) if frame else "") or loc or "value"
        v = ser(value, 64, 2000)
        mem = _mem(value)
        key = json.dumps([v, mem], default=str, sort_keys=True)
        if _last.get(name) == key:
            return value
        _last[name] = key
        send({"kind": "watch", "name": name, "v": v, "mem": mem, "loc": loc, "t": _pytype(value), "caller": _caller.get()})
    except Exception:
        pass
    return value


watch = w


def _pytype(v):
    """Python-flavoured type for containers the JSON can't express (tuple, deque…)."""
    t = type(v).__name__
    return t if t in ("tuple", "deque", "frozenset") else ""


def trace(fn=None, *, name=None):
    """Decorator: records params, args, return value, exceptions, duration."""
    if fn is None:
        return lambda f: trace(f, name=name)
    if OFF or getattr(fn, "__cdev__", False):
        return fn
    label = name or getattr(fn, "__qualname__", getattr(fn, "__name__", "fn"))
    try:
        params = list(inspect.signature(fn).parameters)
    except (TypeError, ValueError):
        params = []
    try:
        loc = f"{os.path.relpath(fn.__code__.co_filename, _CWD)}:{fn.__code__.co_firstlineno}"
    except Exception:
        loc = None

    def snapshot(args, kwargs):
        # taken before the call runs, so later mutation doesn't rewrite history
        try:
            return params[: len(args)] + list(kwargs), [_lite(a) for a in args] + [_lite(v) for v in kwargs.values()]
        except Exception:
            return [], []

    def emit(caller, snap, t0, ret=None, err=None, is_async=False):
        try:
            names, vals = snap
            ev = {"kind": "call", "name": label, "params": names, "args": vals, "loc": loc,
                  "ms": round((time.perf_counter() - t0) * 1000, 3), "caller": caller}
            if err is not None:
                ev["err"] = f"{type(err).__name__}: {err}"
            else:
                ev["ret"] = _lite(ret)
            if is_async:
                ev["async"] = True
            send(ev)
        except Exception:
            pass

    if inspect.iscoroutinefunction(fn):
        @functools.wraps(fn)
        async def awrapper(*args, **kwargs):
            caller = _caller.get()
            snap = snapshot(args, kwargs)
            tok = _caller.set(label)
            t0 = time.perf_counter()
            try:
                r = await fn(*args, **kwargs)
            except BaseException as e:
                emit(caller, snap, t0, err=e, is_async=True)
                raise
            finally:
                _caller.reset(tok)
            emit(caller, snap, t0, ret=r, is_async=True)
            return r
        awrapper.__cdev__ = True
        return awrapper

    @functools.wraps(fn)
    def wrapper(*args, **kwargs):
        caller = _caller.get()
        snap = snapshot(args, kwargs)
        tok = _caller.set(label)
        t0 = time.perf_counter()
        try:
            r = fn(*args, **kwargs)
        except BaseException as e:
            emit(caller, snap, t0, err=e)
            raise
        finally:
            _caller.reset(tok)
        emit(caller, snap, t0, ret=r)
        return r

    wrapper.__cdev__ = True
    return wrapper


def trace_all(cls_or_module, prefix=None):
    """Trace every public function/method on a class or module."""
    pre = prefix or getattr(cls_or_module, "__name__", "")
    for k, v in list(vars(cls_or_module).items()):
        if k.startswith("_"):
            continue
        if inspect.isfunction(v) or inspect.iscoroutinefunction(v):
            setattr(cls_or_module, k, trace(v, name=f"{pre}.{k}" if pre else k))
    return cls_or_module


def log(*args, level="info"):
    loc, _ = _loc(sys._getframe(1))
    send({"kind": "log", "level": level, "text": " ".join(a if isinstance(a, str) else repr(a) for a in args),
          "args": [_lite(a) for a in args], "loc": loc, "caller": _caller.get()})


# ---------------------------------------------------------------- incoming (ASGI / WSGI)


def _hdrs(pairs):
    out = {}
    for k, v in pairs:
        k = k.decode() if isinstance(k, bytes) else k
        v = v.decode(errors="replace") if isinstance(v, bytes) else str(v)
        out[k.lower()] = v[:10] + "…" if k.lower() in ("authorization", "cookie", "set-cookie") else v
    return out


def _body(chunks, headers):
    if not chunks:
        return None
    b = b"".join(chunks)
    ct = headers.get("content-type", "")
    if headers.get("content-encoding") not in (None, "identity"):
        return f"<{headers['content-encoding']} {len(b)} bytes>"
    if ct and not any(t in ct for t in ("json", "text", "xml", "form", "javascript")):
        return f"<{ct.split(';')[0]} {len(b)} bytes>"
    return b[:MAX_BODY].decode(errors="replace")


def asgi(app):
    async def wrapped(scope, receive, send_):
        if scope.get("type") != "http":
            return await app(scope, receive, send_)
        t0 = time.perf_counter()
        req_h = _hdrs(scope.get("headers", []))
        req_chunks, res_chunks, st = [], [], {"status": None, "headers": {}}

        async def recv():
            m = await receive()
            if m.get("type") == "http.request" and sum(map(len, req_chunks)) < MAX_BODY:
                req_chunks.append(m.get("body", b""))
            return m

        async def snd(m):
            if m["type"] == "http.response.start":
                st["status"] = m["status"]
                st["headers"] = _hdrs(m.get("headers", []))
            elif m["type"] == "http.response.body" and sum(map(len, res_chunks)) < MAX_BODY:
                res_chunks.append(m.get("body", b""))
            await send_(m)

        route = f"{scope.get('method')} {scope.get('path')}"
        tok = _caller.set(route)
        err = None
        try:
            await app(scope, recv, snd)
        except BaseException as e:
            err = e
            raise
        finally:
            _caller.reset(tok)
            qs = scope.get("query_string", b"").decode()
            ua = req_h.get("user-agent", "")
            send({"kind": "http", "dir": "in", "method": scope.get("method"),
                  "url": f"http://{req_h.get('host', 'localhost')}{scope.get('path')}{'?' + qs if qs else ''}",
                  "from": req_h.get("x-cdev-from") or ("browser" if "Mozilla" in ua else "client"), "cid": req_h.get("x-cdev-id"), "handler": route,
                  "status": st["status"] or (500 if err else None), "err": str(err) if err else None,
                  "ms": round((time.perf_counter() - t0) * 1000, 2),
                  "req": {"headers": req_h, "body": _body(req_chunks, req_h)},
                  "res": {"headers": st["headers"], "body": _body(res_chunks, st["headers"])}})
    _register_port()
    return wrapped


def wsgi(app):
    def wrapped(environ, start_response):
        t0 = time.perf_counter()
        req_h = {k[5:].replace("_", "-").lower(): v for k, v in environ.items() if k.startswith("HTTP_")}
        if environ.get("CONTENT_TYPE"):
            req_h["content-type"] = environ["CONTENT_TYPE"]
        body = b""
        try:
            n = int(environ.get("CONTENT_LENGTH") or 0)
            if n:
                import io
                body = environ["wsgi.input"].read(n)
                environ["wsgi.input"] = io.BytesIO(body)
        except Exception:
            pass
        st = {}

        def sr(status, headers, exc_info=None):
            st["status"] = int(status.split()[0])
            st["headers"] = _hdrs(headers)
            return start_response(status, headers, exc_info)

        route = f"{environ.get('REQUEST_METHOD')} {environ.get('PATH_INFO')}"
        tok = _caller.set(route)
        chunks = []
        try:
            for c in app(environ, sr):
                if sum(map(len, chunks)) < MAX_BODY:
                    chunks.append(c)
                yield c
        finally:
            _caller.reset(tok)
            qs = environ.get("QUERY_STRING")
            ua = req_h.get("user-agent", "")
            send({"kind": "http", "dir": "in", "method": environ.get("REQUEST_METHOD"),
                  "url": f"http://{req_h.get('host', 'localhost')}{environ.get('PATH_INFO')}{'?' + qs if qs else ''}",
                  "from": req_h.get("x-cdev-from") or ("browser" if "Mozilla" in ua else "client"), "cid": req_h.get("x-cdev-id"), "handler": route,
                  "status": st.get("status"), "ms": round((time.perf_counter() - t0) * 1000, 2),
                  "req": {"headers": req_h, "body": _body([body], req_h)},
                  "res": {"headers": st.get("headers", {}), "body": _body(chunks, st.get("headers", {}))}})
    _register_port()
    return wrapped


def _register_port():
    port = os.environ.get("PORT")
    if port and port.isdigit():
        send({"kind": "service", "name": SVC, "port": int(port)})


# ---------------------------------------------------------------- auto hooks


def _patch_logging():
    """Mirror every record that passes the user's own level checks, without
    adding a handler (which would change basicConfig/lastResort behaviour)."""
    orig = logging.Logger.callHandlers

    def call_handlers(self, record):
        try:
            text = record.getMessage()
            path = record.pathname
            ev = {"kind": "log", "level": record.levelname.lower().replace("critical", "error").replace("warning", "warn"),
                  "text": text, "logger": record.name, "caller": _caller.get(),
                  "loc": f"{os.path.relpath(path, _CWD) if path.startswith(_CWD) else path}:{record.lineno}"}
            if record.exc_info:
                ev["stack"] = "".join(traceback.format_exception(*record.exc_info))
            send(ev)
        except Exception:
            pass
        return orig(self, record)

    logging.Logger.callHandlers = call_handlers


def _patch_http_client():
    C = http.client.HTTPConnection
    orig_request = C.request
    orig_getresponse = C.getresponse

    def request(self, method, url, body=None, headers=None, **kw):
        headers = dict(headers or {})
        if "x-cdev-internal" not in headers:
            scheme = "https" if isinstance(self, http.client.HTTPSConnection) else "http"
            full = url if "://" in url else f"{scheme}://{self.host}{':' + str(self.port) if self.port not in (80, 443) else ''}{url}"
            b = body.decode(errors="replace") if isinstance(body, bytes) else body if isinstance(body, str) else None
            cid = None
            if self.host in ("localhost", "127.0.0.1"):
                cid = os.urandom(5).hex()
                headers["x-cdev-from"] = SVC
                headers["x-cdev-id"] = cid
            self._cdev = {"method": method, "url": full, "t0": time.perf_counter(), "caller": _caller.get(), "cid": cid,
                          "req": {"headers": _hdrs(headers.items()), "body": (b or None) and b[:MAX_BODY]}}
        return orig_request(self, method, url, body=body, headers=headers, **kw)

    def getresponse(self, *a, **kw):
        info = getattr(self, "_cdev", None)
        try:
            resp = orig_getresponse(self, *a, **kw)
        except Exception as e:
            if info:
                send({"kind": "http", "dir": "out", "method": info["method"], "url": info["url"], "caller": info["caller"],
                      "err": str(e), "ms": round((time.perf_counter() - info["t0"]) * 1000, 2), "req": info["req"]})
            raise
        if not info:
            return resp
        self._cdev = None
        base = {"kind": "http", "dir": "out", "method": info["method"], "url": info["url"], "caller": info["caller"], "cid": info["cid"],
                "status": resp.status, "ms": round((time.perf_counter() - info["t0"]) * 1000, 2),
                "req": info["req"], "res": {"headers": _hdrs(resp.getheaders())}}
        done = {"sent": False}

        def fire(body=None):
            if done["sent"]:
                return
            done["sent"] = True
            if body is not None:
                base["res"]["body"] = _body([body], base["res"]["headers"])
            send(base)

        orig_read = resp.read

        def read(amt=None):
            data = orig_read(amt)
            if amt is None:
                fire(data)
            return data

        resp.read = read
        t = threading.Timer(1.5, fire)
        t.daemon = True
        t.start()
        return resp

    C.request = request
    C.getresponse = getresponse


# ---------------------------------------------------------------- auto-watch (no w() calls)

_SKIP_LOCALS = (type(sys), type(_mem), type(len), type, staticmethod, classmethod, property)


def _auto_watch():
    """Trace every function in the project's own files; after each line, send
    what changed: `fn()` = all locals (so {arr, i, j} draws as an array with
    pointers) and `fn.name` for each container/object local."""
    only = {x.strip() for x in os.environ.get("CDEV_ONLY", "").split(",") if x.strip()}
    budget = [int(os.environ.get("CDEV_AUTO_MAX", "20000"))]
    last = {}       # (frame id, name) -> fingerprint
    prev_line = {}  # frame id -> line that ran last (the one that caused a change)
    wanted = {}     # code object -> bool

    def interesting(code):
        r = wanted.get(code)
        if r is None:
            name = code.co_name
            # <frozen importlib…>, <string>, <listcomp>/<genexpr> and dunder methods are noise;
            # --only can still target a dunder explicitly
            if code.co_filename.startswith("<") or (name.startswith("<") and name != "<module>") \
                    or (name.startswith("__") and name.endswith("__") and name not in only):
                wanted[code] = False
                return False
            f = os.path.abspath(code.co_filename)
            rel = f[len(_CWD):] if f.startswith(_CWD) else None
            r = (rel is not None and f != _SELF and "site-packages" not in f
                 and not any(part.startswith(".") for part in rel.split(os.sep))
                 and os.path.basename(f) != "sitecustomize.py"
                 and (not only or code.co_name in only))
            wanted[code] = r
        return r

    def emit(ev):
        if budget[0] <= 0:
            return
        budget[0] -= 1
        if budget[0] == 0:
            sys.settrace(None)
            threading.settrace(None)
            send({"kind": "log", "level": "warn", "text": "cdev auto-watch: event budget used up, tracing stopped (raise CDEV_AUTO_MAX or use --only)"})
        send(ev)

    def snapshot(frame, line):
        code = frame.f_code
        fn = "main" if code.co_name == "<module>" else code.co_name
        fid = id(frame)
        try:
            items = [(k, v) for k, v in frame.f_locals.items()
                     if not k.startswith("__") and k != "cdev" and not isinstance(v, _SKIP_LOCALS) and not callable(v)]
        except Exception:
            return
        if not items:
            return
        rel = os.path.relpath(code.co_filename, _CWD)
        loc = f"{rel}:{line}"
        caller = frame.f_back.f_code.co_name if frame.f_back and interesting(frame.f_back.f_code) else None
        # containers / objects: their own watch (diagrams + memory)
        for k, v in items:
            if isinstance(v, (int, float, str, bytes, bool, type(None), complex)):
                continue
            try:
                sv = ser(v, 64, 2000)
                key = json.dumps(sv, default=str, sort_keys=True)
            except Exception:
                continue
            if last.get((fid, k)) != key:
                last[(fid, k)] = key
                emit({"kind": "watch", "name": f"{fn}.{k}", "v": sv, "mem": _mem(v), "t": _pytype(v),
                      "loc": loc, "caller": fn, "auto": True})
        # the whole frame: primitives + small views of containers
        try:
            fv = {k: ser(v, 3, 200) for k, v in items}
            key = json.dumps(fv, default=str, sort_keys=True)
        except Exception:
            return
        if last.get((fid, "()")) != key:
            last[(fid, "()")] = key
            emit({"kind": "watch", "name": f"{fn}()", "v": fv, "loc": loc, "caller": caller, "auto": True})

    def local(frame, event, arg):
        if event in ("line", "return"):
            fid = id(frame)
            snapshot(frame, prev_line.get(fid, frame.f_lineno))
            prev_line[fid] = frame.f_lineno
            if event == "return":
                # frame ids get reused: forget this frame's state
                prev_line.pop(fid, None)
                for k in [k for k in last if k[0] == fid]:
                    del last[k]
        return local

    def on_call(frame, event, arg):
        if event == "call" and interesting(frame.f_code):
            return local
        return None

    sys.settrace(on_call)
    threading.settrace(on_call)


def _auto():
    if OFF or getattr(_auto, "done", False):
        return
    _auto.done = True
    if os.environ.get("CDEV_AUTO_WATCH"):
        _auto_watch()
    _patch_logging()
    prev = sys.excepthook

    def hook(t, e, tb):
        send({"kind": "error", "msg": f"{t.__name__}: {e}", "stack": "".join(traceback.format_exception(t, e, tb))})
        flush()
        prev(t, e, tb)

    sys.excepthook = hook
    prev_th = threading.excepthook

    def thook(args):
        send({"kind": "error", "msg": f"{args.exc_type.__name__}: {args.exc_value}",
              "stack": "".join(traceback.format_exception(args.exc_type, args.exc_value, args.exc_traceback))})
        prev_th(args)

    threading.excepthook = thook
    try:
        _patch_http_client()
    except Exception:
        pass
