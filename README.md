# cdev

A small, native dev-time sidecar that shows **what your code is actually doing while you write it**:
- values and their types as they're passed around
- data structures drawn as diagrams you can step through
- logs from every process in one stream
- API requests and responses side by side
- a map of what talks to what

It's the lightweight cut of [`docs/dev-suite.md`](docs/dev-suite.md): one 2.7 MB Rust binary, one HTML page, and single-file SDKs for **JS/TS (Node + browser), Python, C++, Rust, Go and PHP**. Your app needs no framework, build step or server.

> Status: early but usable. See [`TODO.md`](TODO.md) for what's done, in progress and planned.

```
┌ your code ─────────────────────────────┐        ┌ cdev (one binary) ───────────────┐
│ Node / TS    cdev-node.js   (auto)     │        │ ingest ─► ring buffer ─┬─► TUI   │
│ Browser      cdev.js        (<script>) │  HTTP  │   :4400                └─► web   │
│ Python       cdev.py        (auto)     │ ─────► │                          panel   │
│ C++          cdev.hpp       (macros)   │  JSON  │ type shapes · structure detection│
│ Rust         cdev.rs        (macros)   │        │ API pairs · call graph · steps   │
│ Go           cdev-go/       (package)  │        │                                  │
│ PHP          cdev.php       (auto)     │        │ /api/events  /api/edges  (JSON)  │
│ any process  stdout/stderr  (cdev run) │        │ :4401 page server (cdev watch)   │
└────────────────────────────────────────┘        └──────────────────────────────────┘
```

## Why

Most debugging is still `console.log`: print a thing, squint, print another thing. cdev replaces that loop.

- **`cdev.w(value)` instead of `console.log(value)`.** It returns the value, so you can wrap any expression in place (`save(cdev.w(user))`). cdev records the value, its type shape (`User { id: number, tags: string[] }`) and the source line, and labels it with the expression text. Every change is kept, so you get a history instead of a single print.
- **`cdev.trace(fn)`** records each call: parameter names, argument values and types *as passed*, return value or thrown error, duration, and who called it.
- **Data structures are drawn, not dumped.** Arrays show index pointers, grids show as tables, linked lists as chains (cycles detected), trees and heaps as trees (with a BST check), graphs as node diagrams, and maps as tables. **Step through the snapshots** with `←`/`→` (or press play in the web panel) and the change at each step is highlighted: `swap [3] ↔ [4]`, `[2]: 5 → 7`, `length 4 → 6`. You're watching the algorithm run.
- **Zero-code capture** under `cdev run`/`cdev watch`: console output, uncaught errors, incoming HTTP (Express, Next, Vite, raw `http`, PHP), outgoing `fetch`/`http`, Python `logging`, and plain stdout/stderr from anything.
- **API view:**
  - **Calls:** every call seen from **both sides**. Top row: the client (request sent ‖ response received, and which function made it). Bottom row: the server (request received ‖ response sent, and which handler). Sides are paired exactly via an `x-cdev-id` header, or by timing when a header can't be added.
  - **Endpoints** (`e`): calls grouped as `GET /users/:id` with count, status breakdown, latency, callers, handlers, and the inferred **request/response body types**.
- **Map view:** services, hosts and traced functions as nodes; edges carry call counts, average latency and errors.
- **Memory view (tab 5):** where each watched value lives (stack / heap / static), its address and size, what it points at, and its heap buffer (`len / cap × elem`). Step through to see what changed: `v: buffer reallocated 0x…c10 → 0x…2b0 (cap 8 → 16)`, `buffer grew in place`, `outgrew its inline buffer`, `head->next inside #12 +8`, `refs 1 → 2`, `⚠ [0], [1], [2] are the same object`.

### Auto-watch (`--auto`): no `w()` calls at all

`cdev --auto watch algo.py` (or `cdev --auto run -- python app.py`, `cdev --auto watch sort.ts`, `cdev --auto watch index.html`) records every function in *your* files. Libraries, `node_modules`, site-packages, imports, comprehensions and dunder methods are skipped. After each line or statement it records what changed:

- **`fn()`**: all of the function's locals as one value. `{arr, i, j, n}` draws as an array with `↑i ↑j` pointers, so plain loops animate without edits.
- **`fn.name`**: one watch per container or object local (`bfs.queue`, `insert.root`), which gets its own diagram and Memory entry.
- Each step's `loc` is the line that caused the change.

Use `--only f,g` to focus, and `CDEV_AUTO_MAX` (default 20000 events) as the safety budget. It's meant for algorithms and scripts, not busy servers.

How it works:
- **Python**: a line tracer (`sys.settrace`).
- **JS / TS**: cdev rewrites your source as it loads, using a native JS/TS parser. It inserts a snapshot call after each statement on the *same line*, so line numbers and stack traces are unchanged, and your files on disk are never touched. Loading is hooked for Node CommonJS, Node ESM, Bun (for `.ts`), and pages served by `cdev watch` (`.js` directly, `.ts` via a Bun build plugin). See the rewrite with `cdev instrument file.ts`.
- **C++ / Rust / Go**: debugger-driven auto-watch is planned; use `CDEV_W` / `cdev_w!` / `cdev.W`.

### Static view (`cdev explain`): read a file without running it

`cdev explain app.ts` opens the **Code** tab on a file that is just sitting there. Nothing is executed and no port is opened. For each function, and for the top level, it shows:

- the signature, parameters and locals, with declared types or the type of a literal initializer (`words: string[] = ['b', 'a']`)
- what it **calls** (resolved to functions in the same file where possible) and what it is **called by**
- what it changes outside itself: a parameter's contents, `this`, or an outer variable; and which outer variables it reads
- the shape of its control flow: loops, branches, returns, throws, awaits, nesting depth
- a one-sentence summary and tags (`recursive`, `self-contained`, `changes its input`, `not used in this file`)
- the function's source underneath

The view re-reads the file when you save it. `cdev watch app.ts` fills the same tab while the file also runs. `cdev explain --text file` prints the report and `--json` gives it to tools (source on stdin with `--name file.ts`).

Limits, since only the syntax is read: values are never known, a call through an object (`obj.method()`) is not resolved to a class, mutation through a method is recognised by name (`push`, `set`, …), and anonymous callbacks are counted as part of the function around them. JS/TS only for now.

### What the Memory view shows per language

| language | you see |
|---|---|
| C++ | real addresses, `sizeof`, stack vs heap vs static, raw/smart pointer targets (`use_count` for `shared_ptr`), `vector`/`string` buffers with `len/cap`, small-string optimisation (chars stored inline), fields inside heap nodes (`inside #3 +8`) |
| Rust | the same via `cdev_w!(&x)` (pass a reference so the address is `x`'s own): `Vec`/`String` `(ptr, len, cap)`, in-place growth vs reallocation, `Box` heap targets, `Rc`/`Arc` strong counts |
| Python | `id()` addresses, `getsizeof`, names → objects, shared objects and the `[[0]*3]*3` aliasing bug |
| JS / TS | no addresses exist, so objects get stable ids (`#3`) showing which names share an object |
| Go | pointer targets, slice headers `(ptr, len, cap)`, string data (`cdev.W(&x)` for `x`'s address) |

Try it: `cdev watch examples/cpp/memory.cpp`, `cdev watch examples/rust/memory.rs` or `cdev watch examples/python/memory.py`, then press `5`.

### Learning a new language with it

cdev works the same way in every language, which makes it a good way to read unfamiliar code. Drop `w()` / `trace()` into a Go, Rust or PHP program and you get the same Vars/API/Map views you already know from JS. Real type names (`&HashMap<i32, usize>`, `std::vector<std::vector<int>>`, `*main.Node`) sit next to the familiar shape, so you learn how each language models things by watching values flow.

## Install

```sh
cargo install --path .        # installs `cdev` into ~/.cargo/bin
```

## Use

```sh
cdev                              # sidecar + TUI; web panel at http://localhost:4400
cdev run -- npm run dev           # wrap a dev server: auto-instrument Node/Python/PHP, capture output
cdev watch index.html             # serve a plain HTML/JS/TS page on :4401, reload on save
cdev watch app.ts                 # run a single file instrumented; re-run on every save
cdev watch algo.py | main.cpp | main.rs | main.go | script.php
cdev explain app.ts               # static view: read a JS/TS file without running it (Code tab)
cdev explain --text app.ts        # …or print the report (--json for tools)
cdev --auto watch algo.py         # auto-watch: record every local after each statement, no cdev.w() needed (Python, JS, TS)
cdev --auto --only bubble_sort watch algo.py   # …only these functions
cdev --headless …                 # no TUI; one line per event on stdout (scripts / agents)
cdev --port 4500 …                # different port (watch pages use port+1)
cdev sdk ./vendor                 # copy all SDK files into a folder
```

**Does my app need a server?** No. cdev is the only server. For a plain `.html` file, either run `cdev watch index.html` (serves it, injects the SDK, live-reloads, compiles `<script src="app.ts">` via bun or esbuild) or open the file directly with `<script src="http://localhost:4400/cdev.js"></script>` in it.

### TUI keys

| key | action |
|---|---|
| `←`/`→` (or `Tab`, `1`–`6`) | switch tab: Live · Vars · API · Map · Mem · Code |
| `e` | API: toggle Calls / Endpoints |
| `↑`/`↓` or `j`/`k` | move selection |
| `[`/`]` or `h`/`l` | **Vars / Mem: step through history** |
| `G` / `End` | jump to newest and follow |
| `Enter` | toggle detail pane |
| `J`/`K` or `PgDn`/`PgUp` | scroll detail |
| `/` | filter (text, kind, or level) |
| `c` | clear |
| `q` | quit (stops the wrapped app) |

The web panel keeps `1`–`5` for tabs and `←`/`→` for stepping, plus `Space` to play/pause in Vars. It has no Code tab yet. Deep links: `/#vars:bubbleSort@4`, `/#api`, `/#map`.

## SDKs

Every SDK does the same things: `w` (watch, returns the value), `trace` (calls), `log`, and batched sending in the background that never blocks or crashes your app. Everything is disabled with `CDEV_OFF=1`.

### Node / TypeScript

Under `cdev run`/`cdev watch` nothing needs to change: the SDK is preloaded and a global `cdev` exists.

```ts
const user = cdev.w(await db.getUser(id));        // label inferred from source: "await db.getUser(id)"
cdev.w({ arr, i, j }, 'sort');                    // array + index pointers → drawn with ↑i ↑j
const getUser = cdev.trace(async function getUser(id: number) { … });
cdev.traceAll(UserService);                       // every method of a class/object
```

Without cdev: `node --require /path/to/cdev-node.js app.js`. Put `cdev.d.ts` (from `cdev sdk`) in the project for types. In bundled browser TS, pass the name explicitly (`cdev.trace(fn, 'countPaths')`), because bundlers can rename functions.

### Browser (React, plain HTML + JS)

```html
<script src="http://localhost:4400/cdev.js"></script>
```

This captures console output, errors, unhandled rejections, and `fetch`/XHR with bodies, and exposes `window.cdev`. In Vite, put the tag in `index.html`. In Next, use `<Script src=… strategy="beforeInteractive" />`.

### Python

```python
import cdev                          # automatic under cdev run / watch
user = cdev.w(get_user(42))
@cdev.trace
def bfs(start, goal): ...
app = cdev.asgi(app)                 # FastAPI / Starlette incoming requests
app = cdev.wsgi(app)                 # Flask / Django
```

Also captured automatically: `logging` (without changing your handlers or levels), uncaught exceptions, and outgoing `http.client`/`urllib` requests.

### C++ (header-only, C++17, POSIX)

```cpp
#include "cdev.hpp"
int total = CDEV_W(price * qty);            // "price * qty" = 42 : int
CDEV_WATCH("quicksort", a);                 // std::vector<int> drawn as an array
void solve(int n, const std::string& s) { CDEV_TRACE(n, s); CDEV_LOG("n is ", n); }
```

`-DCDEV_DISABLE` compiles every macro away.

### Rust (one file, std only)

```rust
#[macro_use] mod cdev;                       // copy cdev.rs into src/
let r = cdev_w!(two_sum(&nums, 9));          // any T: Debug
cdev_w!("reversed", &head);                  // Option<Box<ListNode>> → linked-list diagram
fn solve(n: usize, grid: &Vec<Vec<u8>>) { cdev_trace!(n, grid); cdev_log!("n = {}", n); }
```

No serde needed: the SDK sends `{:?}` output and the sidecar parses it back into structure. Events flush automatically when `main` returns.

### Go (package, stdlib only)

```go
import "cdev"                                 // cdev sdk . → ./cdev-go; go.mod: require cdev v0.0.0 / replace cdev => ./cdev-go
defer cdev.Flush()                            // first line of main
root = cdev.W(insert(root, v), "bst")         // pointers, structs, maps, slices; cycles become refs
func search(xs []int, t int) int { defer cdev.Trace(xs, t)(); … }
http.ListenAndServe(":8080", cdev.Handler(mux))   // incoming; outgoing via http.DefaultTransport is automatic
```

### PHP

```php
require 'cdev.php';                           // automatic under cdev run / watch
$total = cdev_w($price * $qty);
$find = cdev_trace(fn(int $id) => $repo->find($id), 'find');
cdev_log('hello', $user);
```

Also captured automatically: warnings/notices, uncaught exceptions, and each HTTP request with bodies under `php -S` or php-fpm.

### Anything else

POST JSON to `http://127.0.0.1:4400/ingest`: one event, or an array of them.

```sh
curl -d '{"kind":"watch","svc":"sh","name":"x","v":[3,1,2]}' localhost:4400/ingest
```

## Event format

| kind | fields |
|---|---|
| `log` | `level`, `text`, `args[]`, `loc` |
| `watch` | `name`, `v`, `t?` (static type), `loc`, `mem?` |
| `call` | `name`, `params[]`, `args[]`, `argTypes?[]`, `ret`, `err`, `ms`, `caller` |
| `http` | `dir` (`in`/`out`), `method`, `url`, `status`, `ms`, `from`, `caller`, `req{headers,body}`, `res{headers,body}` |
| `error` | `msg`, `stack` |
| `service` | `name`, `port` (maps `localhost:port` → service name on the Map) |

All events also carry `ts`, `svc` and `lang`. The sidecar adds `shape`, `argShapes` and `retShape` (inferred types) and `ds` (the detected data structure). Rust sends `vdebug`/`argsDebug` (`{:?}` text), which becomes `v`/`args`. Shared or cyclic objects are encoded as `{"__ref": n}` pointing at `"__id": n`.

`mem` (optional, on `watch`): `addr`, `size`, `region` (`stack` / `heap` / `static` / `value`), `ptr` + `target_size` + `target_region` for pointers, `refs` for shared ownership, `heap: {addr, len, cap, elem, region}` for buffers (`region: "inline"` = stored inside the object, `"none"` = not allocated yet), and `items[]` / `fields{}` of referenced object addresses (identity/aliasing).

## HTTP API

| route | |
|---|---|
| `GET /` | web panel |
| `GET /cdev.js` | browser SDK |
| `POST /ingest` | events (object or array; any content-type) |
| `GET /api/events?kind=&q=&limit=&since=` | recent events as JSON |
| `GET /api/edges` | architecture edges |
| `GET /api/calls` | HTTP calls with client + server sides paired |
| `GET /api/endpoints` | calls grouped by route with statuses, latency, body types |
| `POST /api/instrument?file=` | JS/TS auto-watch rewrite of the posted source |
| `GET /api/memory?step=` | memory at a step: names, heap blocks, links, change note |
| `GET /stream` | server-sent events of new events |
| `GET /reload` | server-sent reload signals for `cdev watch` pages |
| `POST /api/clear` | clear buffer |

Binds to `127.0.0.1` only. SDKs truncate auth/cookie header values.

## Env

| var | default | |
|---|---|---|
| `CDEV_PORT` | `4400` | sidecar port |
| `CDEV_URL` | `http://127.0.0.1:4400` | where SDKs send events |
| `CDEV_SERVICE` | package / folder name | service name shown in views |
| `CDEV_IGNORE` | static assets, `/@vite`, `/_next/` … | regex of incoming paths the Node SDK skips |
| `CDEV_OFF` | – | disable all SDKs |

## Editor extension (VSCodium / VS Code)

`editor/vscode/` has a dependency-free extension that drives the same binary:
- inline values at the end of lines, and hovers with history
- a "▶ auto-watch" CodeLens above functions
- the panel embedded beside your code: click a location to open it, and step through Vars/Mem to highlight each line

See [`editor/vscode/README.md`](editor/vscode/README.md) to install it.

## Layout

```
src/        main.rs (CLI) · server.rs (HTTP/SSE) · store.rs (ring buffer, views) · event.rs (model, type shapes)
            structure.rs (data-structure detection + TUI diagrams) · memory.rs (memory model per step)
            debugfmt.rs (Rust {:?} parser)
            api.rs (call pairing, endpoints) · instrument.rs (JS/TS auto-watch rewrite)
            explain.rs (JS/TS static view: calls, effects, flow)
            runner.rs (cdev run) · watch.rs (cdev watch) · tui.rs
web/        index.html — the whole web panel
editor/     vscode/ — VSCodium / VS Code extension
sdk/        cdev-node.js · cdev.js · cdev.d.ts · cdev.py · cdev.hpp · cdev.rs · cdev.go · cdev.php
examples/   node-api · web (html+ts) · python · cpp · rust · go · php
```
