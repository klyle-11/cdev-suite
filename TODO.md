# TODO

The lightweight cut of [`docs/dev-suite.md`](docs/dev-suite.md). It is built for use, not as a from-scratch learning exercise, so it leans on `tokio`/`axum`/`ratatui` rather than hand-rolling them.

Legend: ✅ done · 🚧 in progress · 📋 planned · ⚠️ written but untested

---

## ✅ Done

### Sidecar (Rust, one 2.7 MB binary, installed with `cargo install --path .`)
- ✅ Event model + **type-shape inference** in the sidecar (`src/event.rs`): `{ id: number, tags: string[] }`, `User { … }`, `Map<string, number>`, `(number | string)[]`, recursive types as `ListNode { next: ListNode | null }`
- ✅ Ring-buffer store, broadcast, derived views: vars, http, edges (`src/store.rs`)
  - edges count max(client-side, server-side) so each request counts once; routes grouped (`/users/:id`)
  - stdout de-dup of lines an SDK already reported (exact or suffix match, 5 s window)
  - `loc` paths made relative to where cdev was started
- ✅ HTTP server: `/ingest`, `/api/events`, `/api/edges`, `/stream` + `/reload` (SSE), `/api/clear`, `/`, `/cdev.js`; the panel is served with its initial snapshot inlined
- ✅ `cdev run -- <cmd>`: own process group, stdout/stderr → logs, JSON/pino line parsing, ANSI strip, preloads Node (`NODE_OPTIONS`), Python (`sitecustomize`) and PHP (`PHP_INI_SCAN_DIR`), stops the whole app on quit
- ✅ **`cdev watch <file|dir>`** (`src/watch.rs`)
  - HTML/folder: served on port+1 with SDK + reload hook injected, `.ts/.tsx/.jsx` compiled on request (bun → esbuild), CSS-only changes swap in place, other changes reload
  - single files: `.js .ts .py .cpp .rs .go .php` run instrumented and re-run on save (`── run #N ──` markers)
- ✅ CLI: `--port`, `--headless`, `--cap`, `run`, `watch`, `sdk <dir>`
- ✅ **Data-structure detection + diagrams** (`src/structure.rs`): array, array+index pointers, matrix/grid, linked list (cycle + doubly-linked detection), binary tree (size/height/valid-BST), n-ary tree, graph (adjacency list, directed/undirected), hash map / set, heap (array shown as tree); per-step diff notes (`swap [i] ↔ [j]`, `[k]: a → b`, `length n → m`)
- ✅ Rust `{:?}` parser (`src/debugfmt.rs`) so Rust values get shapes + diagrams without serde
- ✅ TUI: Live · Vars (history + ←/→ stepping + diagrams) · API (request ‖ response) · Map (tree of edges), filter, detail pane

### Memory view (step 1)
- ✅ SDKs attach `mem`: C++ (`&v`, `sizeof`, region by stack/static probe, raw + smart pointers, `use_count`, `vector`/`string` buffers, small-string optimisation), Rust (`cdev_w!(&x)`: autoref-specialised probes for `Vec`/`String`/`Box`/`Option<Box>`/`Rc`/`Arc`), Python (`id`, `getsizeof`, element/field identities), JS (stable object ids), Go (pointer/slice/string data, untested)
- ✅ Watch de-dup includes memory, so a move or reallocation with equal values still counts as a step
- ✅ `src/memory.rs` + `/api/memory?step=`: names, heap blocks (buffer / target / object) sorted by address, pointer→name links, containment (`inside #n +off`), sharing counts, per-step notes (reallocated / grew in place / outgrew inline / first allocation / moved / repointed / refs / same object as / aliased items)
- ✅ TUI tab 5 (Mem) with ←/→ stepping; web Mem tab with boxes + SVG arrows, slider, play
- ✅ Serializers treat an array reached twice as shared, not circular (only re-entry on the current path is a cycle)
- ✅ Examples: `examples/cpp/memory.cpp`, `examples/rust/memory.rs`, `examples/python/memory.py`

### API: both sides + endpoints
- ✅ `x-cdev-id` correlation header on local requests in all SDKs (Node http/fetch, browser same-origin fetch, Python http.client, Go transport) + read on the server side (Node, Python ASGI/WSGI, Go, PHP); handler route recorded
- ✅ `src/api.rs`: pairs client/server records (exact by id, else method + path + target + timing), groups endpoints by `METHOD /route/:id` with statuses, avg/max latency, callers, handlers, inferred request/response body types; `/api/calls`, `/api/endpoints`
- ✅ TUI + web: Calls (2×2: client sent/received ‖ server received/sent) and Endpoints (`e`)

### Auto-watch (JS / TS)
- ✅ `src/instrument.rs` (oxc 0.120, pinned for rustc 1.92): same-line insertions only, per-variable getters (TDZ-safe), braces for single-statement bodies, loop-entry snapshots, names for methods / `const f = () =>` / object keys / `arrow:LINE`, idempotent, parse errors → source unchanged
- ✅ `cdev instrument [--name] [file]` and `POST /api/instrument`
- ✅ Runtime (`__cdev_enter` / `__cdev_s`) in Node + browser SDKs; hooks: Node CJS (`Module._compile` → `cdev instrument`), Node ESM (`module.register` loader), Bun (`Bun.plugin`), page server (`.js` rewrite, `.ts` via `cdev-bun-build.js`)
- ✅ Node SDK now flushes at exit (`beforeExit` + blocking fallback on `process.exit`); short scripts used to lose their last events
- ✅ Example: `examples/js/auto.{js,mjs,ts}` (zero cdev calls), verified identical via CJS, ESM and Bun

### Editor extension v1 (`editor/vscode/`)
- ✅ Start / attach / stop, status bar, output channel; "Auto-watch this file" (title button) + "▶ auto-watch fn" CodeLens (py/js/ts); "Watch file"; "Run a command under cdev…"
- ✅ Inline values from the live stream (frame steps show only what changed), hover with type / structure / value / history
- ✅ Panel in an editor tab (iframe of the web panel); click `file:line` → open; Vars/Mem stepping highlights the source line
- ⚠️ Not yet run inside VSCodium here (logic is unit-tested with node; the manifest + commands are unverified in the editor)

### Auto-watch (Python)
- ✅ `--auto` / `--only f,g` flags for `run` and `watch` (`CDEV_AUTO_WATCH`, `CDEV_ONLY`, `CDEV_AUTO_MAX` budget)
- ✅ `sys.settrace` tracer in `cdev.py`: project files only (skips site-packages, frozen importlib, comprehensions, dunders), per-line change detection, `fn()` frame snapshots + `fn.name` container watches, loc = line that caused the change, frame state freed on return
- ✅ Example: `examples/python/auto.py` (bubble sort + BST, zero cdev calls)

### Web panel (`web/index.html`, one file, vanilla JS)
- ✅ Same four views, SSE live updates, light/dark, deep links (`#vars:name@step`)
- ✅ Vars: SVG/HTML diagrams for every structure, step slider, ◀ ▶, **play** (animates the algorithm)
- ✅ Map: layered left→right graph (services, hosts, traced functions), edge weight = count

### SDKs
- ✅ Node `cdev-node.js`: `w`/`trace`/`traceAll`/`log`, console, uncaught errors (non-intrusive), outgoing `http`/`https`/`fetch` + bodies, incoming `http(s).Server` + bodies, port → service registration, AsyncLocalStorage caller tracking, label inference from source, deep serialization with `__ref` for shared/cyclic nodes
- ✅ Browser `cdev.js`: console, errors, rejections, fetch, XHR, `w`/`trace`/`traceAll`
- ✅ Python `cdev.py`: `w`/`trace` (sync + async, args snapshotted before the call), logging mirror (doesn't touch handlers), excepthook, `http.client` hook, `asgi()`/`wsgi()`
- ✅ C++ `cdev.hpp`: `CDEV_W`, `CDEV_WATCH`, `CDEV_TRACE`, `CDEV_LOG`, demangled type names, containers/maps/optional → JSON, background sender, `CDEV_DISABLE`
- ✅ Rust `cdev.rs`: `cdev_w!`, `cdev_trace!`, `cdev_log!`, std only, flush on exit via `atexit`
- ✅ `cdev.d.ts` global typings

### Verified
- ✅ `cargo test`: type shapes, structure detection, swap/cycle/BST notes, Rust Debug parsing, ANSI/pino parsing, TUI renders all tabs (TestBackend)
- ✅ End to end: Node server under `cdev run` (calls, request/response pairs, route-scoped call graph); Python BFS + heap; C++ quicksort via `watch` (compile + re-run on save); Rust linked-list reversal + HashMap via `watch`; HTML + TS page via `watch` (injection, bun compile, CSS)
- ✅ Web panel screenshots checked for Live, Vars (sort/BST/heap), API, Map

## 🚧 In progress

- ⚠️ **Go SDK** (`sdk/cdev.go`, `examples/go`): written, **untested** because Go isn't installed yet. `W[T]`, `Trace(...)()`, `Log`, `Handler`, auto-wrapped `http.DefaultTransport`, reflection serializer with pointer refs
- ⚠️ **PHP SDK** (`sdk/cdev.php`, `examples/php`): written, **untested** because PHP isn't installed yet. `cdev_w`, `cdev_trace`, `cdev_log`, error/exception hooks, per-request capture under `php -S`/fpm
- 🚧 User testing of the TUI and web panel on real projects
- ⚠️ Web Mem tab: logic + API verified, JS syntax-checked; layout not yet checked in a browser (no headless browser here)

## 📋 Planned

**Auto-watch (no `w()` calls needed)**: Python + JS/TS done (see Done), next:
- 📋 Python 3.12+: switch to `sys.monitoring` (much lower overhead than `settrace`)
- 📋 Recursion: distinguish frames of the same function (depth / call id) instead of sharing one `fn()` history
- 📋 `--auto` via a debugger (DAP: `lldb-dap`, `dlv dap`, `debugpy`, Node inspector): step line by line, read locals, auto-continue; one implementation for C++ / Rust / Go
- 📋 JS/TS: inline `<script>` blocks in HTML pages; source maps for tsx/ts-node-compiled CJS (currently instruments the compiled JS)
- 📋 JS/TS: closure variables from enclosing functions (only the function's own params/locals are watched today)

**Memory view (pointers, addresses, bytes)**: step 1 done (see Done), next:
- 📋 Byte view (debugger route): hex dump with field boundaries + padding, struct layout / alignment
- 📋 Stack frames: group names by function frame (needs caller info per watch → frame boxes)
- 📋 C++ pointers into freed memory (track `delete`d targets / allocator hooks)
- 📋 Per-node addresses for Rust `Box`-linked lists (Debug output has no addresses; debugger route)
- 📋 Nested aliasing beyond one level (Python/JS `items`/`fields` are top-level only)

**Learning / DSA**
- 📋 Recursion view: call tree from `trace` caller links (fib/DFS/backtracking), with the ability to step through it
- 📋 Value diff for objects between snapshots (highlight changed keys, not just array cells)
- 📋 Big-O hints: plot call count / duration against input size across runs
- 📋 Pointer labels for linked lists and trees (`slow`, `fast`, `cur` shown on nodes)
- 📋 More structures: trie, union-find (parent array), deque/stack labels, 2D grid paths (BFS frontier coloring)

**Coverage**
- 📋 Timeline scrubber across all views (rewind everything to time T)
- 📋 Source-map resolution for browser stack locations (Vite/Next transformed line numbers)
- 📋 `cdev.w` label inference in the browser (fetch the source from the dev server)
- 📋 Test runner panel: vitest / jest / pytest / cargo test results
- 📋 React helper: `cdev.traceComponent` (renders, props in, why it re-rendered)
- 📋 OTLP/HTTP JSON ingest so OTel-instrumented apps work unchanged
- 📋 Java / C# / Kotlin / Swift SDKs

**Editor extension (next)**
- 📋 Publish to Open VSX; bundle the `cdev` binary per platform
- 📋 Debug-adapter integration: auto-watch C++/Rust/Go through the editor's own debug session
- 📋 Inline memory hints (addresses / realloc notes) and API call CodeLens on route handlers
- 📋 Timeline gutter: step through a function's history from the editor

**Quality of life**
- 📋 `--save session.jsonl` + `cdev replay session.jsonl`
- 📋 Map: collapse function nodes per service; p50/p95 latency per edge
- 📋 Config file (`cdev.toml`) for ignore patterns, buffer size, redaction list
- 📋 Go: per-goroutine caller stacks (currently one shared stack)
- 📋 MCP server so an agent can query the live event log directly

## Design decisions

- **One Rust binary** = sidecar + TUI + web panel + page server. No React, protobuf, gRPC, Docker or CI.
- **Transport:** HTTP POST of JSON arrays to `/ingest`. Any language can speak it; SDKs need no WebSocket client.
- **Web panel:** one static HTML file (vanilla JS + SSE), embedded with `include_str!`.
- **Types and structures are inferred in the sidecar**, so every language renders the same way. Static languages (C++, Rust, Go) also send real type names.
- **Watch records changes only:** SDKs skip a `watch` whose value is identical to the last one under that name, so the history is exactly the algorithm's steps.
- **Deep serialization for watches** (linked nodes don't consume depth); shared/cyclic references become `__ref`/`__id` so cycles are shown instead of crashing.
- **Never change the app's behaviour:** hooks are observation-only (e.g. `uncaughtExceptionMonitor`, Python logging mirrored without adding handlers, promises returned unchanged so unhandled rejections still surface).
- **Neutral naming** throughout the code (e.g. "app" / "caller" / `stop_app`), even where CS convention uses other terms.

## Non-goals (for now)

- Step debugging, DOM inspection, CPU/heap profiling (use DevTools / your IDE)
- Remote attach or auth; binds to localhost only
- Production use; SDKs are dev-only and no-op with `CDEV_OFF`
