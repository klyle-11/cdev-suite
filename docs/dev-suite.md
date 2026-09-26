# dev-suite — Implementation Plan

## Overview

dev-suite is a universal developer observability overlay — a sidecar process that attaches to any running localhost project and surfaces debug state, logs, test output, data flow, and type relationships through two parallel frontends: a browser-injected sidebar panel and a native terminal TUI.

The core insight is that developers context-switch constantly between their editor, browser DevTools, terminal logs, test runner output, and documentation to understand what their code is actually doing at runtime. dev-suite collapses all of that into a single, always-present surface. It is the dev-time equivalent of what Datadog, Grafana, and Splunk do in production — shift-left observability applied to the inner development loop.

The project serves a dual purpose: it is a genuinely useful developer tool, and it is a portfolio artifact that demonstrates SRE-grade architectural thinking — collector patterns, OpenTelemetry protocol support, multi-frontend data distribution, and production observability concepts applied at the development layer.

### Goals

- **Zero-config attachment**: Point dev-suite at any localhost project and get immediate value. No framework-specific plugins required for basic log/variable capture.
- **Language-agnostic collection**: Support TypeScript, Rust, Python, Go, Java, C#, and C++ through a combination of OpenTelemetry Protocol (OTLP) ingestion and lightweight per-language SDK shims.
- **Dual frontend parity**: Browser sidebar and terminal TUI show the same data, rendered appropriately for each medium. SREs live in terminals; frontend devs live in browsers. Serve both.
- **Functional aesthetics**: The matrix-style data waterfall visualizer is not decoration — it is a real-time data flow visualization where streams represent actual payloads, speed maps to throughput, and color encodes type/severity. It doubles as a skeleton loader during connection establishment.
- **Plugin architecture**: The METAL dashboard integrates later as a plugin, proving the system's extensibility.

---

## Relationship to Existing Tools

dev-suite is not trying to replace browser DevTools or your IDE's debugger, and reviewers should not read the panel list and assume otherwise. The honest scope:

### What dev-suite is

A **dev-time observability overlay** for full-stack and polyglot localhost projects. It collapses what is currently spread across browser DevTools, terminal log-tailing, test-runner output, and Postman/network inspectors into one always-visible surface, with **scrubbable history across every dimension of the system simultaneously** (Phase 7.5 — the time-travel scrubber).

The underlying bet is that *most* of what engineers call "debugging" — "why did this variable have the wrong value?", "what happened to that request?", "is this race firing?", "what does the log say?" — is better served by always-on observation with retained history than by stopping execution. This is the same shift production observability went through over the last decade (Splunk → Honeycomb, point-checks → continuous tracing) applied to the inner dev loop.

### Where dev-suite is genuinely better than existing tools

- **Async / race-condition / timing-sensitive bugs.** Pausing a program changes its behavior. Continuous observation doesn't. For "this happens 1 time in 50 under load," the step-debugger is the wrong tool; dev-suite is the right one. Not preference — repro physics.
- **Distributed / polyglot flows.** No debugger spans browser → Node → Python → Postgres. The data flow graph and waterfall do. This is the strongest unique-value case.
- **"What happened?" after the fact.** With a debugger, missing the bug means re-triggering it while paused at the right line. With the scrubbable timeline, you scroll back. Workflow gap that nothing else fills at dev time.
- **Aggregated full-stack logs.** DevTools console sees the browser only. Terminals see one process each. dev-suite aggregates browser console + server stdout/stderr + structured logs (pino/Winston/Python `logging`) + OTLP records in one filterable view.
- **Continuous variable history without breakpoints.** Redux DevTools does this for Redux state; Recoil DevTools for Recoil. There is no generic tool for "watch `userBalance` evolve in my Express app for the next 30 seconds without setting breakpoints." Print debugging is the current answer and it's bad.
- **Terminal-native parity.** SREs work in terminals; no part of the existing browser-tool stack serves them at dev time.

### Where existing tools are still better (and dev-suite does not compete)

- **Step-debugger (VS Code, Chrome DevTools Sources, JetBrains)** wins for: exploratory bug-hunting when you have no hypothesis; walking unfamiliar code or third-party library internals; edit-and-continue / hypotheticals ("what if this value were different?"); seeing the full stack frame at a paused line including variables you didn't instrument.
- **Chrome DevTools Network panel** wins for: HTTP-level request/response debugging, header inspection, request replay. The data flow waterfall treats HTTP as one of many edge types with payloads trimmed for visualization — it is not a Network panel replacement.
- **Chrome DevTools Elements** wins for: anything DOM-related. dev-suite does not touch the DOM inspector space.
- **Chrome DevTools Performance / Memory** wins for: CPU flame charts and heap analysis.
- **React DevTools / Vue DevTools** wins for: framework-specific component trees with props and state. dev-suite is generic; run framework DevTools alongside it.

### Where there is deliberate overlap

- **Console logs.** DevTools console sees browser logs. dev-suite's logs panel also sees browser logs, but with the addition of every server-side and SDK source. Use whichever surface is in front of you; the data is the same.
- **Variable inspection.** DevTools Sources shows scope variables when paused. dev-suite shows watched variables continuously, with history. Different time models, complementary not competing.

### One-line framing

> *Browser DevTools answers "what is true in this browser process at this moment?" dev-suite answers "what has been flowing through my whole system over time?" They are complements, not competitors.*

The target audience is full-stack developers, small polyglot teams, and SREs who live in terminals. Pure-frontend developers working in a single Next.js app will get less out of dev-suite than out of Chrome DevTools + React DevTools alone, and that's an honest scoping statement — not a problem to solve.

---

## Learning Philosophy & Self-Built Approach

dev-suite is built as a learning vehicle as much as a tool. The primary author is using this project to develop real problem-solving instincts in systems programming, data structures and algorithms, and system design — not to ship the fastest possible product by stitching together crates. Wherever a piece of functionality would normally be pulled in as an external dependency, the default is to build it ourselves first, then replace with a battle-tested crate only when the homegrown version has earned its retirement (performance ceiling hit, soundness issues, ecosystem interop required).

This is deliberate. The goal is not NIH (not-invented-here) syndrome — it is to internalise *how* the things we use every day actually work. A developer who has hand-rolled a ring buffer, a broadcast channel, a length-prefixed framing codec, and a force-directed graph layout will reach for the right tool in production with judgment, not cargo-cult.

### Rust as the Learning Vehicle

Coming from JavaScript, Rust forces you to make decisions the GC and the event loop normally make for you: where a value lives, when it dies, who can mutate it, whether two threads can touch it. The compiler refusing to build your code is the language refusing to let you write the bug. The discomfort is the curriculum.

This subsection focuses on the programming concepts you need to read and write the code in this project — syntax, semantics, the standard-library surface, the data structures, and how the abstractions we use every day (futures, channels, broadcast buses, codecs, graph layouts) are actually built underneath.

#### Why Rust Exists (Skim)

Optional context — skip if you're impatient to start coding.

- **C (1972) and C++ (1985)** gave fine-grained memory control with no safety net. Five decades of dangling pointers, use-after-free, buffer overflows, data races followed.
- **The receipts**: Microsoft and Google both disclosed in 2019 that ~70% of their high-severity CVEs are memory-safety bugs. The NSA (2022) and the White House ONCD (2024 *Back to the Building Blocks*) have publicly urged the industry off C/C++.
- **Rust started in 2006** at Mozilla (Graydon Hoare's project, picked up to rebuild Gecko as Servo). Rust 1.0 shipped May 2015 with the claim: *memory safety without GC, data-race freedom without manual locks.* It worked.
- **Now in production at**: AWS Firecracker (Lambda/Fargate VMM), Cloudflare (`quiche` / HTTP/3), Discord (read states), Dropbox (sync engine), Figma (multiplayer), Meta (Sapling), the Linux kernel (since 2022 — the first new kernel language in 31 years), parts of Windows (announced 2024).
- **Implication for this project**: Rust is what the next two decades of systems software will be written in. Building a portfolio-grade Rust project earns hiring signal that is hard to fake.

#### Coming From JavaScript: The Mental-Model Shift

In JavaScript you almost never think about:

- where a value lives (the engine puts it on the heap; references everywhere; GC sweeps it)
- when a value dies (whenever the GC feels like it)
- who is allowed to modify a value (anyone with a reference)
- whether two pieces of code can touch the same value at the same time (single-threaded, so basically never — until you reach for Workers)
- whether a function might silently throw (every function might; you wrap in `try/catch` when you remember to)
- whether a value is "there" (`null`, `undefined`, `NaN`, empty string, `0` — JS conflates absence with several flavours of presence)

Rust forces you to answer every single one of those questions, in writing, before the program will compile. That is what people mean when they say "fighting the borrow checker." It is not fighting the language — it is being made to declare your intent out loud.

The translation table:

| JavaScript thinks... | Rust makes you declare... | Real-life analogy |
|---|---|---|
| "I have a variable, I can pass it anywhere" | Who *owns* this value, and what happens when ownership moves | A library book has exactly one borrower at a time |
| "Objects are passed by reference, primitives by value" | Every type is `Copy` or it is moved; references are explicit (`&T`, `&mut T`) | Lending the book (`&T`) vs. handing it over for good |
| "The GC will clean up eventually" | Values are dropped deterministically when their owner goes out of scope | When you leave the library, the book auto-returns |
| "null/undefined is just a value" | Absence is encoded in the type as `Option<T>`; failure as `Result<T, E>` | The book is on the shelf, or there is a written explanation why it is not |
| "Async runs on the event loop, sprinkle `await`" | Futures are inert state machines; a runtime (tokio) must poll them; `Send`/`Sync`/`Pin` constrain what can cross threads | Futures are recipes; the runtime is the kitchen; the compiler checks ingredients don't spoil mid-dish |
| "Classes have methods, inheritance is how you share" | Structs hold data; traits are capability contracts; no inheritance, only composition + trait objects | A toaster doesn't *inherit from* kitchen-appliance — it *implements* `HeatsBread` |
| "Errors throw and propagate up the stack" | `Result<T, E>` and `?`; the type signature tells you every way a function can fail | An itemised receipt — both what you got and what went wrong — instead of a clerk yelling at a manager somewhere |
| "Mutation is free" | `mut` is opt-in; the compiler enforces "many readers XOR one writer" | A shared whiteboard with one marker — readers crowd around; only the marker-holder writes |
| "I need a queue/map/set, npm has 14 of each" | `Vec`, `VecDeque`, `HashMap`, `HashSet`, `BTreeMap`, `BTreeSet`, channels, `Arc`/`Mutex`/`RwLock` — pick the right one, know the tradeoffs | A small, opinionated toolbox instead of a hardware store with 200 brands of hammer |
| "Just throw it in `node_modules`" | Dependencies are vetted, pinned in `Cargo.lock`, audited with `cargo-audit`/`cargo-deny` | A locked tool cabinet with a sign-in sheet, not an unattended free-for-all |

#### The Core Concepts, Explained Like You're A JS Developer

**Stack vs. heap.** In JS, you don't think about this. In Rust, every value lives in one of two places. The *stack* is your desk — fixed-size things (integers, fixed-size structs, pointers) sit there, get pushed when a function is called, and disappear when it returns. The *heap* is the warehouse out back — variable-size things (`String`, `Vec<T>`, anything `Box`ed) live there, accessed via a pointer on the stack. The GC in JS hides this distinction; in Rust the choice between e.g. `[u8; 64]` (stack array) and `Vec<u8>` (heap vector) is yours and has performance implications. Stack allocation is roughly free; heap allocation walks an allocator and triggers a `Drop` later to free it. Real-world consequence: tight loops that allocate are the #1 source of performance regressions, and Rust makes the cost legible where JS hides it.

**Ownership.** Every value in Rust has exactly one *owner* — the variable binding responsible for cleaning it up when it goes out of scope. Assigning a value to another binding *moves* ownership; the original binding is no longer usable. This is the single biggest mental shift from JS, where `const b = a` gives you two names for the same thing with no implications. In Rust, `let b = a` (for non-`Copy` types) makes `a` unusable. Real-life: there is one physical key to the safe-deposit box. If you give it to your sibling, you no longer have it. If you want both of you to use it, you either (a) make a copy (`.clone()`), (b) lend it temporarily (`&a`), or (c) put it in a shared structure both of you have keys to (`Arc<T>`).

**Borrowing & references.** Lending the key without giving it up. `&T` is an *immutable borrow* (read-only — any number can coexist). `&mut T` is a *mutable borrow* (exclusive write — only one at a time, and no immutable borrows allowed during it). The compiler enforces this statically. "Many readers XOR one writer" — the same rule databases use for transaction isolation, but at the language level, at compile time, with zero runtime overhead. *This is the thing that makes Rust data-race free.*

**Lifetimes.** Every reference has a lifetime — a region of the program during which the referent is guaranteed to be alive. Usually inferred. Sometimes you have to write it down: `fn longest<'a>(x: &'a str, y: &'a str) -> &'a str`. The `'a` is the compiler making you promise that the returned reference will not outlive the inputs. In JS you never think about this because the GC keeps everything alive as long as anyone can see it. In Rust, the compiler keeps a graph of "what references what for how long" and rejects code where a reference could outlive its target. This eliminates use-after-free *at compile time*.

**Move semantics vs. `Copy`.** Some types are cheap enough to copy on assignment — integers, booleans, fixed-size arrays of `Copy` types. These implement the `Copy` trait and behave like JS primitives. Everything else (`String`, `Vec`, custom structs by default) *moves* — assignment transfers ownership and invalidates the source. This is opt-in: derive `Copy` on your own types if all fields are `Copy`, or implement `Clone` and call `.clone()` explicitly for deep copy. The default is move because it is honest — copying a 1 MB `String` is not free and the language refuses to do it implicitly.

**Traits, not classes.** No inheritance. You define `struct`s that hold data, and `trait`s that describe capabilities. A struct can `impl` many traits. Traits can require other traits (`trait Ord: PartialOrd + Eq`). This is composition over inheritance: a `LogBuffer` doesn't *inherit from* a generic buffer — it *implements* `Push`, `Drain`, `Filter`. The wider industry has been converging on this design — Go interfaces, Haskell type classes, Swift protocols all work this way; Java added default methods in 8 (2014); Kotlin made interfaces first-class because inheritance-first was widely considered a mistake by then. Rust never had inheritance to repent of.

**Generics & monomorphisation.** `fn largest<T: PartialOrd>(slice: &[T]) -> &T` is a generic function. At compile time, the compiler generates a specialised copy for every concrete `T` it is called with — *monomorphisation*. Result: zero-runtime-overhead generics. Cost: bigger binaries. Benefit: a `Vec<u8>` is laid out exactly as efficiently as if you had hand-written `VecOfU8`. Important in dev-suite because the ring buffers, codecs, and channels are all generic over their payload type.

**No null. `Option<T>` and `Result<T, E>`.** Tony Hoare invented null in ALGOL W (1965) and publicly apologised for it in 2009 — his "billion-dollar mistake." Rust does not have null. Absence is `None`; presence is `Some(value)`. Fallible operations return `Result<T, E>` — either `Ok(value)` or `Err(error)`. You cannot accidentally ignore an error; the compiler will warn, and idiomatic code uses `?` to propagate. JS's "everything throws, wrap in `try/catch` when you remember" is replaced by "every failure is in the return type."

**Concurrency: `Send`, `Sync`, and "fearless".** Data races (two threads accessing the same memory, at least one writing, with no synchronisation) are impossible in safe code. Mechanism: two marker traits. `Send` means "safe to move to another thread." `Sync` means "safe to share between threads via `&T`." Most types are both. `Rc<T>` is `!Send` because its reference count is not atomic; `Cell<T>` is `!Sync` because interior mutability without locks would be a data race. The compiler enforces these at every thread-spawning boundary. Deadlocks, livelocks, and logical races are still possible — Rust prevents data races (memory-level), not race conditions (logic-level).

**Async/await and the runtime.** Rust ships the *syntax* (`async fn`, `.await`) but not the *runtime*. We bring `tokio`. A future is a state machine — inert until polled by an executor. Unlike JS promises, futures don't start running until something polls them. You compose them with `tokio::join!`, `select!`, etc. Cost: more upfront complexity (`Pin`, `Send` bounds on async functions, cancellation safety). Benefit: zero-cost async, no forced runtime overhead.

**Macros.** Two flavours. Declarative (`macro_rules!`) does pattern-based code generation — `vec![1, 2, 3]` expanding to `Vec::new()` + three `.push()` calls. Procedural macros run actual Rust code at compile time to transform a token stream into another token stream — this is how `#[derive(Debug, Clone)]` works, how `tokio::main` works, how the dev-suite Rust SDK's `#[dev_suite::watch]` will work. Macros replace what other languages do at runtime via reflection.

**`unsafe`.** Escape hatch for things the compiler cannot verify: dereferencing raw pointers, calling FFI/C functions, mutating `static mut`, implementing certain unsafe traits. Inside `unsafe`, *you* promise the invariants hold. Idiomatic Rust encapsulates `unsafe` inside a safe API. `Vec<T>` uses raw pointers internally but presents a 100% safe surface. We will almost certainly not write `unsafe` in dev-suite; we will read it when studying how `Vec` and `tokio::sync::broadcast` are built.

#### The Standard-Library Surface (What Replaces `[]`, `{}`, and `Map`)

| You reached for in JS | Rust equivalent | Notes |
|---|---|---|
| `[]` (array literal) | `Vec<T>` (heap, growable) or `[T; N]` (stack, fixed) | The split is the cost-vs-flexibility tradeoff JS hides |
| `[].push/.pop/.shift/.unshift` | `Vec` for push/pop at end; `VecDeque` for push/pop at both ends | Choosing wrong is an O(n) vs. O(1) bug |
| `{}` / `Map` | `HashMap<K, V>` (unordered, fast) or `BTreeMap<K, V>` (ordered, slightly slower) | JS `Map` is iteration-ordered; Rust forces you to pick |
| `Set` | `HashSet<T>` or `BTreeSet<T>` | Same split |
| `null` / `undefined` | `Option<T>` | The type signature tells callers absence is possible |
| `try/catch` | `Result<T, E>` + `?` | Errors live in the return type, not as runaway exceptions |
| Promises | `Future<Output = T>` + an executor | Lazy, not eager |
| `Object.freeze` | The default. Mutation requires `mut` | Inverse of JS |
| `JSON.stringify` / `JSON.parse` | `serde_json::to_string` / `from_str` | `serde` is the canonical serialization framework |
| Shared mutable state across threads | `Arc<Mutex<T>>` or `Arc<RwLock<T>>` | Cheaper read-heavy reads via `RwLock`; lock-free via `dashmap`/atomics |
| Single-threaded shared state | `Rc<RefCell<T>>` | `RefCell` panics at runtime on borrow violations; better than nothing |
| Channels (`postMessage`) | `tokio::sync::mpsc` / `broadcast` / `oneshot` / `watch` | Four flavours covering one-to-one, one-to-many, fire-once, latest-value |

In dev-suite, every one of these has at least one usage in the codebase, and several have hand-rolled "from scratch" implementations before the stdlib version is adopted, so the tradeoffs become muscle memory.

#### Building The Abstractions From The Ground Up

The point of this project is to internalise *how* the libraries we use are built. For each major abstraction in dev-suite, the plan is: write a minimal version yourself, get it working, then read the production crate's source to see what you missed and replace your version.

- **`Vec<T>` (growable array).** Write `MyVec<T>` over a raw heap allocation: capacity, length, pointer; `push` that doubles on full; `Drop` that frees. Then read `std::vec::Vec` and note: niche optimisations, `realloc` vs alloc-copy-free, `ManuallyDrop` for unwinding safety.
- **`VecDeque<T>` (ring buffer).** Two-pointer ring over a `Vec`; wraparound arithmetic; `push_front`, `push_back`, `pop_*`. Used directly by `log_buffer.rs` and `variable_store.rs`. Read `std::collections::VecDeque` for the power-of-two-capacity trick that turns modulo into bitmask.
- **`HashMap<K, V>`.** Open addressing with linear probing first, then Robin Hood hashing. Hash trait, equality, load factor, growth strategy. Read `hashbrown` (the Swiss Table used inside stdlib) to see SIMD probing.
- **MPSC / broadcast channel.** Start with `Arc<Mutex<VecDeque<T>>>` + a `Condvar`. Then lock-free with atomics. Then read `tokio::sync::broadcast` for the slow-subscriber semantics (lagged receivers, drop-oldest, per-subscriber position).
- **Async runtime (toy version).** A single-threaded executor: poll a `Future<Output = ()>`, return when `Pending`, store the waker, resume when woken. Not for production, just to demystify `tokio::spawn` — it's exactly this with a work-stealing thread pool, an I/O reactor (epoll/kqueue/IOCP), and timers.
- **Length-prefixed framing codec.** Read a varint, then read that many bytes, then decode. Handle partial reads from the socket. Then read `tokio_util::codec::LengthDelimitedCodec` to see what production looks like.
- **Force-directed graph layout.** Vectors, springs, Coulomb repulsion, Verlet integration, Barnes-Hut quadtree for the O(n²) → O(n log n) speedup. Then look at `d3-force` to see the API decisions a mature library makes.
- **Virtualised list.** Track scroll offset, item heights, render only the visible window plus an overscan. Then look at `react-window` for measurement caches and dynamic-height handling.
- **Reconnecting WebSocket client.** State machine: `Disconnected` → `Connecting` → `Open` → `Closing` → backoff with jitter. Buffer outgoing messages during outage with a bounded queue and overflow policy.

The Dependency List below remains the *eventual* target. Treat it as a menu of things to study and reproduce, not a shopping list to install on day one. Each dependency adoption gets a one-paragraph note in the commit message: what was tried first, what failed, what the dependency provides that the homegrown version could not.

#### Live Debates (Skim)

Optional context — useful in interviews and code review, not required to start coding. One sentence each.

- **Async runtime fragmentation.** `tokio` dominates but isn't universal — libraries hard-coded to it can't be used in `async-std` / `smol` / `embassy` without adapters. Runtime-agnostic traits are in progress.
- **`async fn` in traits.** Was impossible without `async-trait` (boxes every future) until Rust 1.75 (Dec 2023) stabilised native async fn in traits. `Send` bounds on returned futures are still an active design question.
- **Effect / "keyword" generics.** Proposal to make functions polymorphic over `async`-ness, `const`-ness, fallibility. Polarising. Experimental.
- **Compile times.** The biggest fair complaint. Mitigations: `mold`/`lld`, `cargo-chef`, `sccache`, Cranelift codegen backend, Polonius (incremental borrow checker).
- **`unsafe` and soundness.** High-profile crate soundness bugs (`actix-web` 2020, the `Pin`-projection saga) raised the bar. `unsafe_op_in_unsafe_fn` is now default-warn. Miri (UB detector) is becoming a CI standard.
- **"Rewrite It In Rust" backlash.** Healthy pushback. RIIR makes sense when memory safety matters, GC pauses are unacceptable, or the existing implementation is small and you have Rust expertise. Otherwise it's procrastination dressed as engineering.
- **Supply-chain risk.** `crates.io` has had typosquat/malware incidents (the 2022 "rustdecimal" attack). Mitigations: `cargo-deny`, `cargo-vet`, `cargo-audit`. Better than npm, not invincible.

#### Interview Spot-Checks

Ten questions a senior interviewer uses to separate "read the Rust Book" from "shipped Rust." Each one is a thing you should be able to answer because you've *done* the underlying work in this project.

1. **`let s = String::from("hello"); let t = s;` — why can't you use `s` afterwards?** Move semantics: the `(ptr, len, cap)` triple on the stack is moved to `t`; the heap allocation is now `t`'s; using `s` would risk a double-free.
2. **`Box<T>` vs `Rc<T>` vs `Arc<T>` vs `RefCell<T>` vs `Mutex<T>`.** Single-owner heap, single-threaded shared, thread-safe shared (atomic refcount), single-threaded runtime-checked interior mutability, thread-safe interior mutability. Know the (shared? mutable? threaded?) matrix.
3. **`Send` vs `Sync`. Give a type that's `Send` but not `Sync`.** `Cell<T>`. Interior mutability without locks is fine to *move* across threads, not to *share* by reference.
4. **What changed about async-in-traits in Rust 1.75?** Native `async fn` in traits stabilised (with caveats on `Send` bounds).
5. **What is `Pin<T>`?** Type-system primitive saying "this value cannot be moved out of its location." Necessary for self-referential futures.
6. **How does `tokio::spawn` actually run a future?** Executors, the reactor pattern, work-stealing schedulers — and why blocking calls inside `async fn` ruin everything.
7. **When have you reached for `unsafe`?** "I haven't needed to" is a good answer. Casual `unsafe` is a red flag.
8. **`HashMap<String, Vec<Event>>`, ten threads writing to different keys — how do you lock?** A single `Mutex<HashMap>` serialises all writers (wrong). `RwLock` helps reads. `dashmap` shards. Or restructure: per-thread maps merged periodically. The question tests whether you model contention.
9. **Rust vs Go for a backend service.** Rust when predictable latency / CPU efficiency / compile-time correctness / `no_std`/embedded/WASM matter. Go when developer velocity, large rotating teams, I/O-bound workloads, or Go-specific ecosystems (k8s, Terraform). Wrong answer: "Rust is always better."
10. **A Rust pattern you wish other languages had.** Exhaustive `match` on enums; `Result` + `?`; the newtype pattern for type-safe IDs; the typestate pattern for compile-time state machines; `Drop`/RAII for deterministic cleanup.

#### Honest Caveats

Real costs, said plainly.

- **Learning curve is steep.** The borrow checker is the language refusing to let you write the bug. Takes 2–3 months to feel like a feature instead of a wall. Plan for slow before fast.
- **Compile times.** Cargo builds get slow on large projects. Tools help; perfection doesn't exist yet.
- **Younger ecosystem.** Web frameworks, ML, GUI in Rust are usable but not as mature as Java/Python equivalents. Changing fast.
- **Hiring is harder both ways.** Fewer Rust engineers in the pool — and that cuts in your favour if *you* are one.
- **Async Rust is genuinely hard.** Building the event bus in this project is partly an exercise in earning that intuition.

#### What This Means For Day-To-Day Work On dev-suite

The phase guides (`/generate-phase-guide`) will introduce concepts as the code needs them. Because the project is now front-loaded with visual prototypes in TypeScript, Rust enters in the middle of the plan:

- **Phases 1–7 (frontend prototypes)** → TypeScript only. The mock data shapes will be the same as the eventual protobuf message types, so the migration to live data later is mechanical, not redesign.
- **Phase 8 (Rust workspace + protocol)** → `cargo`, crates, modules, `Cargo.toml`, `prost`, derive macros, generic functions, `serde`.
- **Phase 9 (sidecar foundation)** → `async fn`, `tokio`, `axum` extractors, `?` for error propagation, `tokio::sync::broadcast`.
- **Phase 11 (sidecar collectors)** → ownership, borrowing, lifetimes through real data structures; `Arc<Mutex<_>>` vs `RwLock` vs `dashmap`; channels for fan-out; ring buffers and adjacency-list graphs by hand.
- **Phase 14 (TUI + shared waterfall crate)** → arena allocation, `no_std`-compatible code, a shared crate compiled to both native and WASM.
- **Phase 15 (OTLP)** → trait objects, dynamic dispatch, gRPC streaming, `Pin` in practice.
- **Phase 16 (Rust SDK)** → procedural macros, `tracing` integration, `Send`/`Sync` bounds on user-supplied closures.

The goal at the end of the project is: *I have written, debugged, and reasoned about the building blocks (ring buffers, broadcast channels, async futures, codecs, force-directed layouts) myself, so when I reach for the production version of any of them I know what trade-offs I just bought.*

### Data Structures Built From Scratch

Each of the following appears in the dependency list as a convenience, but the *first* implementation in this project is hand-rolled. The crate is introduced later only if benchmarks or correctness demand it.

| Data structure / mechanism | Where it lives | Why we build it ourselves |
|---|---|---|
| **Ring buffer (bounded log/event history)** | `log_buffer.rs`, `variable_store.rs` | Forces a decision about overwrite vs. drop, capacity vs. allocation strategy, and exposes the index/iteration tradeoffs that `VecDeque` papers over |
| **Broadcast / pub-sub channel** | `bus/event_bus.rs` | Hand-rolled MPMC over `Arc<Mutex<...>>` first, then a lock-free version, *then* `tokio::sync::broadcast`. Teaches backpressure, slow-subscriber handling, and lagged-receiver semantics |
| **Adjacency-list graph + traversal** | `data_flow.rs`, `type_registry.rs` | BFS/DFS, topological order, cycle detection, sliding-window edge sets — the DSA fundamentals that interview prep skims and real systems rely on |
| **Force-directed graph layout** | `ui/src/panels/TypesPanel/TypeGraph.tsx` | Build a Barnes-Hut-style simulation before reaching for `d3-force`. Vectors, springs, repulsion, damping — physics-by-code |
| **Length-prefixed binary framing codec** | `protocol/codec.rs`, `ui/src/protocol/codec.ts` | Hand-write the framing layer over the WebSocket before introducing `protobuf`/`prost`. Partial reads, varints, endian decisions |
| **Trie / prefix index for log search** | `log_buffer.rs` | Substring search across 10k+ entries at interactive latency — a real reason to know your indexing structures |
| **LRU cache for type-metadata lookups** | `type_registry.rs` | Doubly-linked list + HashMap, classic interview question, genuinely useful here |
| **Concurrent map (sharded HashMap)** | `variable_store.rs` | Build a sharded `Mutex<HashMap<_, _>>` before importing `dashmap`. Teaches contention measurement |
| **Async stream multiplexer** | `server/websocket.rs` | Fan-out one event source to many WebSocket subscribers with per-subscriber backpressure. The core abstraction `tokio::sync::broadcast` provides — first as an exercise, then as a replacement |
| **Character-grid renderer (matrix waterfall)** | `dev-suite-waterfall` | Custom rendering primitive shared between TUI (`ratatui::Buffer`) and WASM. No external dep does this for us — this *is* the work |
| **CLI argument parser** | `main.rs` | Parse `clap`-style subcommands by hand once, then adopt `clap`. You only respect `clap` after writing your own positional/flag parser |
| **Reconnecting WebSocket client w/ exponential backoff** | `ui/src/protocol/client.ts` | Backoff with jitter, connection-state machine, message buffering during outage. Every distributed-system primer covers this; few junior engineers have written it |
| **Virtualized list rendering** | `ui/src/panels/LogsPanel/LogStream.tsx` | Build the windowing logic before reaching for `react-window`. Forces you to confront measurement, scroll restoration, and overscan |

The dependency list in the Tech Stack section above remains the *eventual* target — but treat it as a menu of things to study and reproduce, not a shopping list to install on day one. Each dependency adoption must be justified with a one-paragraph note in the commit message: what was tried first, what failed, what the dependency provides that we could not.

### System Design Concepts Surfaced By This Project

dev-suite intentionally exposes the author to the system-design vocabulary that production engineering interviews and on-call rotations both demand:

- **Collector / sidecar architecture** (Fluentd, OTel Collector, Envoy) — a process whose job is to receive, aggregate, and fan out telemetry
- **Backpressure** — what happens when producers outrun consumers. Drop? Buffer? Block? We will hit this for real in the broadcast channel
- **Event sourcing & ring-buffer-as-source-of-truth** — the sidecar's log/variable history is a bounded event log
- **CQRS-lite split** — write path (ingestion) vs. read path (panel queries) have different shapes
- **Protocol design** — versioning, forward/backward compatibility, framing, schema evolution. The decision to start with hand-rolled JSON and migrate to protobuf is itself a system-design lesson
- **Push vs. pull** — WebSocket push to the browser vs. Prometheus-style pull from `/metrics`
- **Graceful degradation** — SDK behaviour when the sidecar is absent
- **Observability of the observability tool** — meta, but the right exercise
- **Plugin / extension boundaries** — the METAL plugin work in Phase 13 forces a real plugin-API design

### Workflow: Plan, Generate, Iterate

This document is the *root* plan. It is intentionally long and intentionally not the only artifact. The day-to-day development loop uses two Claude Code skills:

- **`/plan-project`** — used at the very start (this document is its output) and re-invoked whenever the scope of the project shifts materially (a new phase appears, a phase is descoped, a major architectural decision is revisited). Edits to this file flow back through `/plan-project` rather than being free-hand rewrites, so the structure stays consistent.
- **`/generate-phase-guide`** — used at the start of each Implementation Phase to produce a focused, learning-oriented guide for that phase. The phase guide expands the bullet-level steps in this document into the concrete syntax, data-structure choices, and design tradeoffs the author needs to learn *to actually build that phase*. Each phase guide is its own file (e.g. `phase-02-core-sidecar.md`) and is the working document for that phase's commits.

The intended cadence: open the next phase → run `/generate-phase-guide` → work through the guide → mark phase complete → update this root plan with any deviations (scope changes, deferred items, dependency adoptions) → optionally re-run `/plan-project` if the deviations were structural → move on.

This plan is therefore a *living document*. Strikethroughs, "deferred to Phase X" notes, and dated edits in-line are expected and welcomed. The Success Criteria checklist at the bottom is the contract; everything between here and there is allowed to evolve.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                        TARGET APPLICATION                           │
│   (Any localhost project: Next.js, Express, Flask, Go, Rust, etc.) │
│                                                                     │
│   ┌──────────────┐  ┌──────────────┐  ┌──────────────────────────┐ │
│   │ OTLP SDK     │  │ dev-suite SDK│  │ stdout/stderr capture    │ │
│   │ (standard)   │  │ (lightweight)│  │ (process wrapper)        │ │
│   └──────┬───────┘  └──────┬───────┘  └────────────┬─────────────┘ │
└──────────┼─────────────────┼───────────────────────┼───────────────┘
           │ gRPC/HTTP       │ WebSocket/JSON        │ pipe/pty
           │ :4317/:4318     │ :4400                 │
           ▼                 ▼                       ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     RUST SIDECAR (dev-suite-core)                   │
│                          :4400 (primary)                            │
│                                                                     │
│   ┌─────────────┐  ┌──────────────┐  ┌────────────────────────┐   │
│   │ OTLP        │  │ Custom Proto │  │ Process Manager        │   │
│   │ Receiver     │  │ Receiver     │  │ (stdout/stderr/pty)    │   │
│   └──────┬──────┘  └──────┬───────┘  └────────────┬───────────┘   │
│          │                │                       │                │
│          ▼                ▼                       ▼                │
│   ┌─────────────────────────────────────────────────────────────┐  │
│   │                    DATA AGGREGATION LAYER                    │  │
│   │                                                              │  │
│   │  ┌──────────┐ ┌──────────┐ ┌───────────┐ ┌──────────────┐  │  │
│   │  │ Variable │ │ Log      │ │ Data Flow │ │ Type/OOP     │  │  │
│   │  │ Store    │ │ Buffer   │ │ Graph     │ │ Registry     │  │  │
│   │  └──────────┘ └──────────┘ └───────────┘ └──────────────┘  │  │
│   │                                                              │  │
│   │  ┌──────────────┐  ┌──────────────────────────────────────┐ │  │
│   │  │ Test Runner  │  │ Event Bus (tokio broadcast channels) │ │  │
│   │  │ Aggregator   │  │                                      │ │  │
│   │  └──────────────┘  └──────────────────────────────────────┘ │  │
│   └─────────────────────────────────────────────────────────────┘  │
│                                                                     │
│   ┌──────────────────────┐    ┌──────────────────────────────┐    │
│   │ WebSocket Server     │    │ TUI Renderer (ratatui)       │    │
│   │ (serves sidebar app  │    │ (direct terminal output)     │    │
│   │  + streams data)     │    │                              │    │
│   │ :4400/ws             │    │                              │    │
│   └──────────┬───────────┘    └──────────────────────────────┘    │
└──────────────┼────────────────────────────────────────────────────┘
               │
               ▼
┌──────────────────────────────────────────────┐
│          BROWSER SIDEBAR (React)              │
│   Injected via bookmarklet / script tag       │
│   Loads from :4400, renders in shadow DOM     │
│                                               │
│   ┌────────┐ ┌────────┐ ┌──────────────────┐ │
│   │ Debug  │ │ Logs   │ │ Data Flow        │ │
│   │ Vars   │ │ Panel  │ │ Waterfall (GL)   │ │
│   ├────────┤ ├────────┤ ├──────────────────┤ │
│   │ Type   │ │ Test   │ │ METAL Plugin     │ │
│   │ Graph  │ │ Output │ │ (future)         │ │
│   └────────┘ └────────┘ └──────────────────┘ │
└──────────────────────────────────────────────┘
```

### Component Breakdown

- **Rust Sidecar (`dev-suite-core`)**: The heart. A single Rust binary that receives telemetry via OTLP and a custom JSON-over-WebSocket protocol, aggregates it into typed data stores, and distributes it to both frontends. Also hosts the TUI directly. Built with `tokio` for async I/O, `tonic` for gRPC (OTLP), `axum` for HTTP/WebSocket, and `ratatui` for the TUI.
- **React Sidebar (`dev-suite-ui`)**: A React SPA served by the sidecar on `:4400`. Injected into the target app's page via a bookmarklet or `<script>` tag. Renders in a shadow DOM panel to avoid CSS conflicts. Connects back to the sidecar via WebSocket for real-time data streaming.
- **Language SDKs (`dev-suite-sdk-*`)**: Lightweight per-language shims that instrument applications to emit dev-suite's custom protocol events (variable snapshots, type metadata, test runner hooks). The TypeScript SDK ships first. For languages with mature OTLP support, the SDK wraps and extends the standard OTel SDK rather than replacing it.
- **Process Wrapper**: An alternative collection mode where dev-suite wraps the target process (`dev-suite run -- npm run dev`), capturing stdout/stderr and injecting environment variables for SDK auto-configuration.

---

## Non-Functional Requirements

### Performance

- Sidecar must add < 5ms latency to any instrumented operation
- TUI renders at ≥ 30fps with < 16ms frame time even with 10,000+ log entries in buffer
- Browser sidebar must not block the target application's main thread (Web Worker for data processing)
- WebSocket message throughput: sustain 10,000 events/second without backpressure
- Memory: sidecar stays under 100MB RSS with default buffer sizes (configurable ring buffers)

### Security

- All communication is localhost-only by default (bind to 127.0.0.1)
- Optional token-based authentication for remote attachment (future)
- SDK shims must be zero-overhead in production builds (compile-time feature flags in Rust, tree-shakeable in TypeScript, conditional imports in Python)
- No sensitive data leaves the machine — this is a local-only tool
- Dependency supply chain: pin all Cargo.lock and package-lock.json, audit with `cargo-audit` and `npm audit`

### Scalability

- Configurable ring buffer sizes per data channel (default 10,000 entries for logs, 1,000 for variable snapshots)
- Horizontal scaling is not applicable (this is a single-machine dev tool), but the architecture (collector → event bus → subscribers) mirrors production patterns for educational value
- Plugin system allows adding new data panels without modifying core

### Maintainability

- Rust workspace with clear crate boundaries (core, protocol, tui, sdks)
- React app follows feature-slice architecture (each panel is a self-contained module)
- Shared protocol definitions via Protocol Buffers (generates Rust and TypeScript types from a single source)
- Strict TypeScript (no `any`), Rust clippy on pedantic

### Reliability

- Sidecar crash must never crash the target application (all communication is async, fire-and-forget from the SDK side)
- Graceful degradation: if the sidecar is not running, SDKs become no-ops
- TUI and browser sidebar reconnect automatically on sidecar restart

### Testability

- Test Coverage Targets: Unit 80%+, Integration 60%+, E2E for critical paths
- Testing Approach: Pragmatic — tests written alongside code, not strictly test-first
- Test Automation: CI-integrated via Jenkins pipeline
- Test Data Strategy: Factories and deterministic generators for telemetry events
- Rust: `#[cfg(test)]` modules with `tokio::test` for async, `proptest` for property-based testing of protocol serialization
- TypeScript: Vitest for unit/integration, Playwright for sidebar E2E

### Observability

- The tool that observes other tools must itself be observable: structured logging via `tracing` crate with JSON output
- Internal metrics: events received/second, buffer utilization, WebSocket connection count, frame render time
- Health endpoint at `:4400/health` returning JSON status

### DevOps

- CI/CD: Gitea (self-hosted Git) + Jenkins (pipeline automation)
- Containers: Multi-stage Docker builds for the sidecar binary (Rust builder → scratch/distroless)
- IaC: Terraform for any AWS demo infrastructure
- Branch strategy: trunk-based development with short-lived feature branches
- Security scanning: `cargo-audit`, `cargo-deny`, `npm audit`, Trivy for container images

---

## Tech Stack

| Layer | Technology | Justification |
|---|---|---|
| Sidecar Runtime | Rust + Tokio | Zero-cost abstractions, async I/O, single binary deployment, memory safety without GC — critical for a tool that must add zero perceptible overhead |
| gRPC / OTLP | tonic | De facto Rust gRPC library, native OTLP protocol support, async with tokio |
| HTTP / WebSocket | axum | Tower-based, composable middleware, native WebSocket upgrade, same tokio runtime |
| TUI Framework | ratatui + crossterm | Immediate-mode rendering, sub-millisecond frame times, rich widget library, active ecosystem (0.30.0+, no_std capable) |
| Protocol Definition | Protocol Buffers (protobuf) | Single source of truth for message types, generates Rust (`prost`) and TypeScript (`protobuf-ts`) bindings, forward-compatible schema evolution |
| Browser Sidebar | React 19 + TypeScript | Component model fits panel-based UI, massive ecosystem, aligns with frontend skill set |
| Sidebar Bundler | Vite | Fast dev builds, tree-shaking, single-file output for injection |
| Sidebar Styling | Tailwind CSS (inside shadow DOM) | Utility-first, no style leakage via shadow DOM encapsulation, rapid prototyping |
| Data Viz (Browser) | WebGL / Canvas 2D + Three.js | Matrix waterfall visualizer requires GPU-accelerated rendering for smooth animation at high data rates |
| Data Viz (TUI) | ratatui widgets + custom canvas | Sparklines, bar charts, and custom character-grid rendering for terminal waterfall effect |
| State Management | Zustand | Minimal boilerplate, works well with WebSocket streaming, no provider wrapping |
| TS SDK | TypeScript (npm package) | First-class SDK for the most common target language |
| Python SDK | Python (pip package) | Lightweight shim using `threading` for async event emission |
| Unit Testing (Rust) | built-in `#[test]` + tokio::test | Standard Rust testing, no external framework needed |
| Property Testing | proptest | Fuzz protocol serialization/deserialization round-trips |
| Unit Testing (TS) | Vitest | Fast, ESM-native, compatible with React Testing Library |
| E2E Testing | Playwright | Cross-browser sidebar injection testing |
| CI/CD Platform | Gitea + Jenkins | Self-hosted, enterprise-relevant, demonstrates infra independence from GitHub/Azure |
| IaC Tool | Terraform | Industry standard, AWS provider mature, aligns with AWS CCP cert |
| Container Runtime | Docker (multi-stage) | Reproducible builds, minimal final images using scratch/distroless base |
| Logging (Internal) | tracing crate (Rust) | Structured, async-aware, span-based — the same philosophy the tool teaches users |
| Monitoring (Internal) | Prometheus metrics endpoint | `:4400/metrics` exposes sidecar health; dogfooding observability |
| SAST | cargo-clippy (pedantic), ESLint | Catches both Rust and TypeScript issues at the lint stage |
| Dependency Scanning | cargo-audit, cargo-deny, npm audit | Supply chain security for both ecosystems |
| Container Scanning | Trivy | Open-source, CI-friendly, scans both container images and filesystems |
| Secret Scanning | gitleaks | Pre-commit hook + CI pipeline stage |

---

## Key Technical Decisions & Industry Context

### Decision 1: Visual-First Prototyping with Mock Fixtures

- **Underlying bet of the project**: observation-as-debugging > pause-and-inspect-as-debugging for distributed, async, polyglot, and after-the-fact-investigation work. The step-debugger keeps winning for exploratory bug-hunting and library-internal walkthroughs, and dev-suite does not try to replace it. But the project bets that *most* of what engineers actually call "debugging" — "why did this variable have the wrong value?", "what happened to that request?", "is this race condition firing?", "what does the log say?" — is better served by always-on observation with scrubbable history (see Phase 7.5) than by stopping execution. The Visual-First decision below is what protects that bet, because building the UI first forces the team to confirm the observation model is actually usable before any sidecar code is written.
- **What we chose**: Build the entire browser frontend (sidebar shell + four primary panels) to production visual quality against hand-authored mock fixtures *before* writing any Rust sidecar, protocol, or live data wiring. The mock fixture types are the contract; the eventual protobuf schema in Phase 8 must match them.
- **What we considered**: (1) **Bottom-up** — build the sidecar foundation, protocol, and collectors first, then the UI on top. The orthodox systems approach. (2) **Vertical slices** — build one feature end-to-end (collector → protocol → store → panel) before starting the next. (3) **Visual-first with mocks** (chosen) — every panel is finished visually, in Storybook and a host page, before the backend exists.
- **The active debate**: Bottom-up is the default for systems projects because protocol/state changes are expensive to retrofit. Vertical-slice (championed by agile/XP, popular at Basecamp/37signals via Shape Up) keeps every increment shippable. Visual-first (championed by design-led teams: Figma, Linear, Vercel) treats the UI as the spec — if you can't see it, you don't know what you're building. The risk of visual-first is that the mock shape underspecifies real-world data (cardinality, error states, partial failures); the mitigation is making the fixture set adversarial — empty, idle, error storm, slow trickle, stress.
- **What a tech lead would ask**: "How do you avoid throwing away the mocks?" Answer: the fixtures are typed against the same shape the eventual protobuf will generate. In Phase 10, the Zustand stores grow a `dataSource: 'fixture' | 'websocket'` toggle; nothing else in the panel code changes. The fixtures keep earning their keep as Storybook scenarios and as test data for unit + E2E.
- **Why this fits the author**: Frontend-first matches the author's preference recorded in CLAUDE memory (build UI/visuals first, then wire up backend) and lets the visual design lock in before deeper Rust/systems work consumes attention.
- **Where this is heading**: Tool-as-design-artifact (Linear, Raycast, Arc) is winning the design-led-product battle. Treating the UI as the unmovable spec and the backend as the part that bends to fit it has become a viable strategy when the team has strong frontend taste and weaker backend constraints — which describes this project.

### Decision 2: Sidecar Architecture over Browser Extension or Framework Plugin

- **What we chose**: A standalone Rust process that communicates via WebSocket and gRPC, serving both a browser sidebar and a native TUI.
- **What we considered**: (1) Chrome DevTools extension — gives DOM access but locks you into Chromium and can't serve a TUI. (2) Framework-specific plugins (Vite plugin, Next.js DevTools, Express middleware) — deep access but per-framework maintenance burden and no path to language-agnostic support. (3) Reverse proxy (BrowserSync model) — low friction but limited to HTTP-visible data, no runtime introspection.
- **The active debate**: The industry is split between "deep integration" (Nuxt DevTools, React DevTools, Angular DevTools — framework-specific, maximal insight) and "universal observability" (OpenTelemetry, Jaeger, Zipkin — language-agnostic, standards-based). Nobody has successfully bridged these at the dev-time layer. Framework DevTools teams (led by Anthony Fu for Nuxt, the React team at Meta) argue that deep framework knowledge yields better DX. The OTel community (Charity Majors at Honeycomb, Ben Sigelman at Lightstep/ServiceNow, Ted Young and Austin Parker driving OTel's specification) argues that standards prevent vendor lock-in and scale across polyglot stacks. dev-suite takes the OTel side for the collection protocol but adds custom SDK extensions for the deep introspection that OTLP alone can't provide — a pragmatic middle ground.
- **What a tech lead would ask**: "How do you handle framework-specific state (React component tree, Vue reactivity graph) without framework plugins?" Answer: we don't, initially. The custom SDK provides generic variable snapshots and type metadata. Framework-specific adapters are a future plugin concern, not a core architecture decision. This is YAGNI applied correctly.
- **Where this is heading**: The convergence point is likely OpenTelemetry for Development (OTel DevEx SIG is actively working on this). The Grafana team's Faro project already pushes browser telemetry to OTel backends. Within 1-2 years, expect framework DevTools to expose their data via OTLP, making dev-suite's collection layer forward-compatible.
- **Historical context**: DevTools started as Firebug (Joe Hewitt, 2006), got absorbed into browsers, then forked into framework-specific tools (React DevTools 2015, Vue DevTools 2016). The sidecar/collector pattern comes from production observability: Fluentd (2011), OpenTelemetry Collector (2019), Vector (2020, Rust-based). dev-suite applies the production pattern to the dev loop.

### Decision 3: Rust for the Sidecar, not Node.js or Go

- **What we chose**: Rust (tokio async runtime)
- **What we considered**: (1) Node.js — natural fit for TypeScript-heavy projects, but single-threaded event loop creates backpressure risk at high event rates, and V8 memory overhead conflicts with "zero overhead" goal. (2) Go — excellent for network services, goroutines handle concurrency well, but garbage collector introduces unpredictable latency spikes. (3) Python — ruled out immediately for a performance-critical data pipeline.
- **The active debate**: The "Rust for infrastructure tools" trend is well-established (Ripgrep, fd, bat, delta, Alacritty, Vector, OpenObserve) and accelerating, but the Rust learning curve and compile times are real costs. Go advocates (including many in the Kubernetes ecosystem, led by Kelsey Hightower's philosophy) argue that Go's simplicity yields faster iteration. The Rust community (driven by figures like Andrew Gallant, BurntSushi, and the async working group) counters that zero-cost abstractions and ownership semantics eliminate entire classes of bugs. For dev-suite, Rust wins because: (a) it must be zero-overhead, (b) it hosts the TUI directly in-process (ratatui is Rust-native), and (c) it is a portfolio differentiator in SRE hiring.
- **What a tech lead would ask**: "What's your compile time story? Rust builds can be slow." Answer: Cargo workspace with incremental compilation, `cargo-watch` for dev loop, `mold` linker on Linux, and crate boundaries designed to minimize recompilation cascades. The TUI crate depends on the protocol crate but not on the OTLP receiver crate, so UI iteration doesn't trigger full rebuilds.
- **Where this is heading**: Rust in observability is accelerating. OpenObserve (Rust-based, positions as 10x more efficient than Elasticsearch), Vector (Rust, acquired by Datadog), Quickwit (Rust, distributed search engine for logs). The Rust Foundation is investing in ecosystem maturity. ratatui hit 0.30.0 with `no_std` support and growing adoption (lazygit-class tools). The bet on Rust for tooling is being validated repeatedly.

### Decision 4: Dual Frontend (React Sidebar + ratatui TUI)

- **What we chose**: Two separate rendering targets sharing the same data layer via WebSocket (browser) and in-process channels (TUI).
- **What we considered**: (1) Single frontend — browser only (simpler but ignores terminal-native SRE workflows). (2) Electron app (cross-platform GUI, but heavy, and sidebars don't need desktop chrome). (3) Tauri app (Rust + web frontend, lighter than Electron, but still a standalone window rather than an overlay). (4) Shared rendering via ratzilla (Ratatui → WASM → browser) — promising for the matrix waterfall but premature for full UI parity.
- **The active debate**: Tauri (Daniel Thompson-Nicola and team) is challenging Electron's dominance for desktop apps, but dev-suite isn't a desktop app — it's an overlay. The TUI renaissance (lazygit, k9s, lazydocker, bottom, gitui — all ratatui or bubbletea) proves that terminal UIs are not a niche; they're the preferred interface for infrastructure engineers. Having both frontends signals understanding of both audiences.
- **What a tech lead would ask**: "How do you prevent feature drift between the two frontends?" Answer: the sidecar's event bus is the single source of truth. Both frontends subscribe to the same typed event stream. The protocol crate defines every message shape. Parity is enforced by the data contract, not by duplicating rendering logic. Visual parity is not the goal — appropriate rendering per medium is.
- **Phase placement (per Decision 1)**: The browser frontend is built first to visual completion (Phases 1–7) so the design language is locked before TUI work starts. The TUI ships as a single phase (Phase 14) after the sidecar exists, sharing the same in-process event bus the browser reaches over WebSocket. Building both at once would have doubled the design surface while neither was finished; staggering them keeps the visual bar honest.
- **Where this is heading**: ratzilla (Ratatui → WASM → browser) is actively being developed and could eventually allow rendering the TUI layout directly in the browser as a third rendering mode. This is noted as a future exploration, not a current dependency.

### Decision 5: Gitea + Jenkins over GitHub Actions

- **What we chose**: Self-hosted Gitea for Git hosting, Jenkins for CI/CD pipelines.
- **What we considered**: (1) GitHub + GitHub Actions — most common, great DX, but Microsoft-owned and cloud-dependent. (2) GitLab CE — self-hosted option but heavier than Gitea. (3) Forgejo — Gitea fork with different governance, viable but less proven.
- **The active debate**: The industry defaults to GitHub Actions for convenience, but enterprise SRE teams frequently manage self-hosted infrastructure. Jenkins, despite being "old," remains the most deployed CI/CD platform globally (used by 44% of organizations per the 2024 State of DevOps). Knowing Jenkins signals enterprise readiness. Gitea is lean (single Go binary, ~40MB RAM) and has Forgejo as a governance safety net if the Gitea project direction changes.
- **What a tech lead would ask**: "Why not GitHub Actions? It's where most open-source contribution happens." Answer: This is a portfolio project. Demonstrating self-hosted Git infrastructure + Jenkins pipeline configuration shows a deeper level of infrastructure competency than a `.github/workflows/` YAML file. The project can always mirror to GitHub for visibility.
- **Phase placement (per Decision 1 / incremental-MVP preference)**: Gitea + Jenkins is deferred to Phase 19 — not because it's unimportant, but because spinning it up during Phase 1 burns time on infrastructure for a prototype that hasn't earned the discipline yet. Local builds, lint, and tests are sufficient through the visual-prototype phases. Once a working end-to-end vertical exists (Phase 13), CI/CD is worth the setup cost. This trades early CI hygiene for faster time-to-first-demo, consistent with the user's "demo-able MVP before CI/CD" preference.
- **Where this is heading**: Gitea Actions (GitHub Actions-compatible runner) and Forgejo's CI are closing the DX gap. Jenkins is being modernized with Jenkins X and CloudBees CI. The self-hosted Git movement is growing, partly driven by Microsoft/GitHub policy concerns, partly by sovereignty requirements in regulated industries.

### Decision 6: Matrix Waterfall as Functional Data Visualization

- **What we chose**: A WebGL/Canvas-rendered cascading data stream visualization where visual properties encode real telemetry data, implemented as both a primary data flow view and a skeleton/connection-state indicator.
- **What we considered**: (1) Static dependency graphs (D3 force-directed) — useful but doesn't show temporal flow. (2) Flame charts (Chrome DevTools style) — great for profiling but doesn't map well to ongoing data streams. (3) Sankey diagrams — good for flow volume but hard to read in real-time. (4) Plain log scrolling — functional but not a portfolio differentiator.
- **The active debate**: Data visualization in observability is dominated by time-series charts (Grafana), flame graphs (Brendan Gregg's work, now standard in most profilers), and service maps (Datadog, Jaeger). The "animated stream" approach is less common in professional tools (closest precedent: Weave Scope's animated network visualization, now archived). The risk is that it looks gimmicky. The mitigation is making every visual property meaningful: column = data channel, speed = throughput, color = type/severity, character content = truncated payload, density = event rate. If the visualization answers questions ("which channel is busiest? where are the errors? is the system idle or saturated?"), it's functional. If it just looks cool, it's decoration.
- **What a tech lead would ask**: "What's the performance cost of rendering this at 60fps with thousands of events?" Answer: WebGL for the browser (GPU-accelerated, offloaded from main thread via OffscreenCanvas in a Worker). Custom character-grid rendering in the TUI (ratatui's Buffer is already a character grid — the waterfall is just writing styled characters into it at positions that update each frame, which is O(rows × columns) per frame, trivially fast).
- **Phase placement (per Decision 1)**: The browser waterfall is built in Phase 5 against `dataFlowFixtures` — that means the design and the rendering pipeline are validated *before* the sidecar's data-flow graph exists. The visual phase forces the renderer to be honest about its data dependency: it consumes a typed stream of events, not "whatever the backend hands us." When the TUI variant ships in Phase 14, both renderers share the `dev-suite-waterfall` crate, which has been compileable to WASM from day one against the same fixture shapes.
- **Where this is heading**: GPU-accelerated terminal rendering (Alacritty, WezTerm, Ghostty) is making rich TUI animations viable. On the web side, WebGPU is replacing WebGL for compute-heavy visualization. The broader trend is making observability more visual and less text-dump-oriented — dev-suite rides this wave.

---

## Project Structure

```
dev-suite/
├── Cargo.toml                          # Workspace root
├── Cargo.lock
├── Jenkinsfile                         # CI/CD pipeline definition
├── Dockerfile                          # Multi-stage build for sidecar
├── docker-compose.yml                  # Local dev: Gitea + Jenkins + sidecar
├── terraform/                          # AWS demo infrastructure
│   ├── main.tf
│   ├── variables.tf
│   └── outputs.tf
├── proto/                              # Protocol Buffer definitions (shared)
│   ├── dev_suite.proto                 # Custom protocol messages
│   └── buf.yaml                        # Buf schema registry config
│
├── crates/                             # Rust workspace members
│   ├── dev-suite-core/                 # Main sidecar binary
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs                 # Entry point, CLI args, startup
│   │       ├── config.rs               # Configuration loading
│   │       ├── server/
│   │       │   ├── mod.rs
│   │       │   ├── http.rs             # Axum HTTP server (serves sidebar, health, metrics)
│   │       │   ├── websocket.rs        # WebSocket upgrade + message routing
│   │       │   └── otlp.rs             # OTLP gRPC/HTTP receiver (tonic)
│   │       ├── collector/
│   │       │   ├── mod.rs
│   │       │   ├── variable_store.rs   # Debug variable snapshot storage
│   │       │   ├── log_buffer.rs       # Ring buffer for log entries
│   │       │   ├── data_flow.rs        # Data flow graph construction
│   │       │   ├── type_registry.rs    # Type/OOP metadata registry
│   │       │   └── test_aggregator.rs  # Test runner result aggregation
│   │       ├── bus/
│   │       │   ├── mod.rs
│   │       │   └── event_bus.rs        # Tokio broadcast channel hub
│   │       └── process/
│   │           ├── mod.rs
│   │           └── wrapper.rs          # Process wrapping for stdout/stderr capture
│   │
│   ├── dev-suite-tui/                  # TUI frontend (ratatui)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── app.rs                  # TUI application state
│   │       ├── ui/
│   │       │   ├── mod.rs
│   │       │   ├── layout.rs           # Panel layout (tabs, splits)
│   │       │   ├── variables_panel.rs  # Debug variables view
│   │       │   ├── logs_panel.rs       # Log viewer with filtering
│   │       │   ├── dataflow_panel.rs   # Matrix waterfall in terminal
│   │       │   ├── types_panel.rs      # Type/OOP graph (ASCII art)
│   │       │   ├── tests_panel.rs      # Test runner output
│   │       │   └── status_bar.rs       # Connection status, event rate
│   │       └── events.rs              # Keyboard/mouse event handling
│   │
│   ├── dev-suite-protocol/             # Shared protocol types
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── messages.rs             # Rust types generated from protobuf
│   │       └── codec.rs               # Serialization/deserialization
│   │
│   └── dev-suite-waterfall/            # Matrix waterfall renderer (shared Rust)
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── stream.rs               # Data stream column model
│           ├── renderer.rs             # Character grid renderer (TUI-compatible)
│           └── wasm.rs                 # WASM bindings for browser (future: ratzilla)
│
├── ui/                                 # React browser sidebar
│   ├── package.json
│   ├── tsconfig.json
│   ├── vite.config.ts
│   ├── index.html
│   └── src/
│       ├── main.tsx                    # Entry point
│       ├── App.tsx                     # Root layout with panel tabs
│       ├── inject.ts                   # Bookmarklet / script tag injection logic
│       ├── protocol/
│       │   ├── types.ts               # TypeScript types from protobuf
│       │   ├── client.ts              # WebSocket client with reconnection
│       │   └── codec.ts              # Protobuf encode/decode
│       ├── stores/
│       │   ├── variableStore.ts       # Zustand store for debug variables
│       │   ├── logStore.ts            # Zustand store for log entries
│       │   ├── dataFlowStore.ts       # Zustand store for data flow events
│       │   ├── typeStore.ts           # Zustand store for type metadata
│       │   └── testStore.ts           # Zustand store for test results
│       ├── panels/
│       │   ├── VariablesPanel/
│       │   │   ├── index.tsx
│       │   │   ├── VariableTree.tsx
│       │   │   └── VariableInspector.tsx
│       │   ├── LogsPanel/
│       │   │   ├── index.tsx
│       │   │   ├── LogStream.tsx
│       │   │   └── LogFilter.tsx
│       │   ├── DataFlowPanel/
│       │   │   ├── index.tsx
│       │   │   └── WaterfallCanvas.tsx  # WebGL matrix waterfall
│       │   ├── TypesPanel/
│       │   │   ├── index.tsx
│       │   │   └── TypeGraph.tsx        # D3/Canvas type dependency graph
│       │   └── TestsPanel/
│       │       ├── index.tsx
│       │       └── TestResultTree.tsx
│       ├── components/
│       │   ├── Sidebar.tsx             # Main sidebar shell (shadow DOM host)
│       │   ├── PanelTabs.tsx           # Tab navigation between panels
│       │   └── ConnectionStatus.tsx    # Sidecar connection indicator
│       └── waterfall/
│           ├── WaterfallRenderer.ts    # WebGL rendering engine
│           ├── StreamColumn.ts         # Single data stream column
│           └── shaders/               # GLSL vertex/fragment shaders
│               ├── waterfall.vert
│               └── waterfall.frag
│
├── sdks/                               # Per-language SDK shims
│   ├── typescript/                     # @dev-suite/sdk
│   │   ├── package.json
│   │   ├── tsconfig.json
│   │   └── src/
│   │       ├── index.ts
│   │       ├── client.ts              # WebSocket connection to sidecar
│   │       ├── variables.ts           # Variable snapshot instrumentation
│   │       ├── types.ts               # Type metadata extraction (TS compiler API)
│   │       └── testing.ts             # Test runner hooks (Vitest, Jest)
│   │
│   ├── python/                         # dev-suite-sdk (pip)
│   │   ├── pyproject.toml
│   │   └── src/
│   │       └── dev_suite/
│   │           ├── __init__.py
│   │           ├── client.py          # WebSocket client (threading-based)
│   │           ├── variables.py       # Variable snapshot via inspect module
│   │           └── testing.py         # pytest plugin hooks
│   │
│   └── rust/                           # dev-suite-sdk (crate)
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── client.rs              # Async WebSocket client
│           └── macros.rs              # Procedural macros for instrumentation
│
├── examples/                           # Demo target applications
│   ├── express-app/                    # Node.js/Express instrumented example
│   ├── fastapi-app/                    # Python/FastAPI instrumented example
│   └── axum-app/                       # Rust/Axum instrumented example
│
└── docs/
    ├── ARCHITECTURE.md                 # Detailed architecture documentation
    ├── PROTOCOL.md                     # Protocol specification
    ├── SDK_GUIDE.md                    # How to instrument your app
    └── PLUGIN_API.md                   # Plugin system documentation (METAL integration)
```

---

## Implementation Phases

**Ordering principle.** This project is built **visual-first**: every UI panel is implemented to its finished design against mock data before any Rust sidecar, protocol, or live data wiring is written. The goal is to lock down what the tool *looks like and feels like* before investing in the systems work to feed it real data. Mock fixtures are shaped to match the eventual protobuf messages, so the swap to live data later is mechanical.

The four phase groups, in order:

- **Part A — Visual Prototypes (Phases 1–7)**: TypeScript only. Browser sidebar shell + all four primary panels (Variables, Logs, Data Flow / Matrix Waterfall, Types), polished against mocked fixtures, in Storybook and in a host page.
- **Part B — Sidecar & Live Wiring (Phases 8–13)**: Rust workspace, protocol, sidecar foundation, collectors, TypeScript SDK, Tests panel as the first fully end-to-end vertical.
- **Part C — TUI (Phase 14)**: Terminal frontend parity, sharing the sidecar's event bus in-process.
- **Part D — Ecosystem & Ops (Phases 15–20)**: OTLP, additional language SDKs, plugin system + METAL, testing pyramid, DevOps, documentation.

---

### Phase 1: Frontend Scaffold & Mock Fixtures

The React/TypeScript foundation, the build pipeline for single-bundle shadow-DOM injection, and the mock data fixtures that every visual phase will render against.

#### Step 1.1: Initialize React Sidebar App

- **Command**: `npm create vite@latest ui -- --template react-ts` inside the `ui/` directory
- **Purpose**: Scaffold the browser sidebar with Vite + React + TypeScript. Configure for single-file output (library mode in Vite) so the sidebar can be injected as a single script.
- **Expected Output**: `npm run dev` serves a blank React app; `npm run build` produces a single JS bundle

#### Step 1.2: Frontend Dev Tooling

- **Commands**: Configure ESLint flat config, Prettier, Tailwind CSS in shadow-DOM mode, Vitest for unit tests, Storybook for visual component iteration
- **Purpose**: Fast iteration loop. Vite HMR for live editing, Storybook for designing panels in isolation against fixtures, Vitest watching alongside.
- **Expected Output**: `npm run lint`, `npm run test`, `npm run storybook`, `npm run dev` all pass / boot cleanly

#### Step 1.3: Single-Bundle Library Build

- **Files to Create**: `ui/vite.config.ts` configured for `build.lib` mode emitting a single self-contained bundle suitable for `<script>` injection
- **Purpose**: Future-proof the build pipeline so the shadow-DOM-injected sidebar can be loaded from the sidecar's static-file route in Phase 9 with no further changes.

#### Step 1.4: Mock Fixtures Module

- **Files to Create**: `ui/src/__mocks__/fixtures.ts`, plus one fixture file per data type (`variableFixtures.ts`, `logFixtures.ts`, `dataFlowFixtures.ts`, `typeFixtures.ts`, `testFixtures.ts`)
- **Purpose**: Hand-authored, scenario-rich sample data — quiet system, heavy load, error storms, slow trickle, idle state. Every fixture is typed to match the *eventual* protobuf message shape (defined informally in TypeScript now, formalised in Phase 8). The fixture types are the single source of truth for the visual phases.
- **Why this matters**: Every panel in Phases 3–6 renders against these fixtures and against a `fixtureClock` that can replay events at controllable rates. When live data lands in Phase 10, swapping the Zustand store's data source from `fixture` to `websocket` is the only change.

---

### Phase 2: Browser Sidebar Shell

The sidebar appears in the host page, injected via shadow DOM, with the five-panel tab layout and a connection status indicator driven (for now) by a mock toggle in the dev harness. This is the first visual milestone.

#### Step 2.1: Shadow DOM Injection

- **Files to Create**: `ui/src/inject.ts`, `ui/src/components/Sidebar.tsx`
- **Purpose**: The sidebar renders inside a shadow DOM attached to a `<div>` injected into a host page. This prevents CSS conflicts. For now, a tiny `dev-host.html` page included in the Vite dev server loads the bundle directly so the injection path is exercised from day one.
- **Styling approach**: Tailwind classes inside the shadow DOM. The shadow boundary isolates styles bidirectionally.

#### Step 2.2: Panel Tab Layout

- **Files to Create**: `ui/src/App.tsx`, `ui/src/components/PanelTabs.tsx`, `ui/src/components/ConnectionStatus.tsx`, stub `index.tsx` for each of the five panels in `ui/src/panels/*/`
- **Purpose**: Tabbed interface with five panels: Variables, Logs, Data Flow, Types, Tests. Connection status indicator (green/amber/red) wired to a `useConnectionStatus()` hook backed by a mock store — toggle from a dev-only debug menu to drive the visual states.

#### Step 2.3: Empty / Loading / Error States

- **Files to Create**: `ui/src/components/PanelState.tsx`, shared placeholder components
- **Purpose**: Define the three out-of-data states each panel will use — "awaiting data" skeleton, "loading" shimmer, "error" with retry CTA. Building these up front means each subsequent panel inherits the design language rather than reinventing it.

---

### Phase 3: Variables Panel (Visual)

The Variables panel built to its finished design, rendered against `variableFixtures`. This is the visual depth check: if this looks right, the design system is working.

#### Step 3.1: Variable Store (Frontend)

- **Files to Create**: `ui/src/stores/variableStore.ts`
- **Purpose**: Zustand store seeded from `variableFixtures`. Exposes `subscribe`, `getScope`, `getHistory(path)`. The fixture clock pumps simulated `VariableSnapshot` events into the store at a configurable rate.

#### Step 3.2: Variable Tree + Inspector

- **Files to Create**: `ui/src/panels/VariablesPanel/index.tsx`, `VariableTree.tsx`, `VariableInspector.tsx`
- **Purpose**: Tree view of variable scopes (expandable). Each row shows name, current value, type, and a change indicator (flash highlight on update). Clicking opens the inspector with full value, type info, and a history sparkline.
- **Styling**: Monospace values, color-coded types (string=green, number=blue, object=amber, error=red). Change flash via CSS transition.

#### Step 3.3: Per-Variable History Scrubber

- **Files to Create**: `ui/src/panels/VariablesPanel/HistoryScrubber.tsx`
- **Purpose**: When a variable is selected in the inspector, render a horizontal timeline of its snapshots. Click or drag to scrub through the history — the inspector value updates to that point in time. This is the per-variable version of the cross-panel timeline scrubber that lands in Phase 7.5.
- **Why this matters**: This is the visible payoff of the project's underlying bet (Decision 1, observation-over-pause-debugging) — "what was this variable five seconds ago, and what was it ten seconds ago?" without setting breakpoints and re-running.

#### Step 3.4: Storybook Coverage

- **Files to Create**: `ui/src/panels/VariablesPanel/VariablesPanel.stories.tsx`
- **Purpose**: One story per fixture scenario (empty, single scope, deep nesting, rapid updates, error values, history-scrub interaction). Visual baseline for the panel — what the rest of the panels' polish bar will be measured against.

---

### Phase 4: Logs Panel (Visual)

Virtualised, filterable log stream rendered against `logFixtures`. Performance test: must stay smooth with 10,000 entries replayed at 100 events/sec.

#### Step 4.1: Log Store + Fixture Clock

- **Files to Create**: `ui/src/stores/logStore.ts`
- **Purpose**: Ring-buffered log store (hand-rolled before any `react-window` dependency). Configurable max entries. Batched updates (one frame's worth) to avoid React storms.

#### Step 4.2: Virtualised Log Stream

- **Files to Create**: `ui/src/panels/LogsPanel/index.tsx`, `LogStream.tsx`, `LogFilter.tsx`
- **Purpose**: Hand-roll the windowing first — track scroll offset, item heights, overscan. Auto-scroll to bottom unless the user has scrolled up. Color-coded severity, timestamp column, expandable structured fields on click.
- **Filter bar**: level, source, free-text search. Filter logic runs in the store, not the component.

#### Step 4.3: Storybook + Stress Stories

- **Files to Create**: `LogsPanel.stories.tsx`
- **Purpose**: Stories for: empty, mixed-level normal traffic, error storm, long single-line entries, expanded structured fields, filter combinations. One "stress" story replays a 100k-entry fixture to check frame time.

---

### Phase 5: Data Flow + Matrix Waterfall (Visual)

The portfolio showpiece. WebGL-rendered cascading data streams, against `dataFlowFixtures`. This is where the design language sells the project.

#### Step 5.1: Stream Column Model

- **Files to Create**: `ui/src/waterfall/StreamColumn.ts`
- **Purpose**: Pure TypeScript model of a single column: identity, throughput, recent event ring buffer, current character grid state. No rendering here — testable in isolation.

#### Step 5.2: WebGL Renderer

- **Files to Create**: `ui/src/waterfall/WaterfallRenderer.ts`, GLSL shaders in `ui/src/waterfall/shaders/`
- **Purpose**: WebGL-rendered cascading streams. Column width ∝ throughput, fall speed ∝ event rate, colour encodes type/severity (green=success, amber=slow, red=error, blue=type metadata, white=neutral). Hover freezes a column; click drills into payload detail.
- **Architecture**: OffscreenCanvas in a Web Worker for rendering, transferring frames to the main thread. Prevents the waterfall from competing with the host app's UI thread.

#### Step 5.3: Skeleton Loader Mode

- **Files to Create**: integrated into `WaterfallRenderer.ts`
- **Purpose**: Before any data source is connected, the waterfall renders with dim, randomised placeholder characters. When live data starts (or in the visual phase: when the fixture clock starts), streams crossfade — the "coming alive" moment. This doubles as the connection-state skeleton across the whole sidebar.

#### Step 5.4: Data Flow Panel Container

- **Files to Create**: `ui/src/panels/DataFlowPanel/index.tsx`, `ui/src/stores/dataFlowStore.ts`
- **Purpose**: Wire the renderer to the data-flow store, which is fed from `dataFlowFixtures` via the fixture clock. Inspector pane shows the selected stream's recent events.

---

### Phase 6: Types Panel (Visual)

Interactive force-directed type-relationship graph, against `typeFixtures`.

#### Step 6.1: Force Simulation (Hand-Rolled)

- **Files to Create**: `ui/src/panels/TypesPanel/forceSim.ts`
- **Purpose**: Build the simulation by hand before reaching for `d3-force`. Vectors, springs (attraction along edges), Coulomb-style repulsion between nodes, damping. Verlet integration. A Barnes-Hut quadtree for the O(n log n) repulsion approximation once node counts get past ~50.

#### Step 6.2: Type Graph Renderer

- **Files to Create**: `ui/src/panels/TypesPanel/TypeGraph.tsx`
- **Purpose**: Canvas-based render of nodes + edges. Nodes sized by field/method count. Edge style distinguishes inheritance (solid), composition (dashed), generic instantiation (dotted). Click a node → full type definition panel. Search/filter by name.

#### Step 6.3: Type Store + Storybook

- **Files to Create**: `ui/src/stores/typeStore.ts`, `TypesPanel.stories.tsx`
- **Purpose**: Store seeded from `typeFixtures`. Stories for small graph, large graph, dense interconnection, disconnected components.

---

### Phase 7: Visual Polish & Design Freeze

The bar after this phase: every panel is at production visual quality against mock data. No further visual changes should be required after live data lands in Phase 10.

#### Step 7.1: Theme + Tokens

- **Files to Create**: `ui/src/theme/tokens.ts`, Tailwind theme extension
- **Purpose**: Centralised color, spacing, typography tokens. Dark mode is the primary theme; a light variant ships at the same time so the contrast story is honest.

#### Step 7.2: Empty / Error / Loading State Audit

- **Purpose**: Walk every panel through the three states using the dev harness toggles. Fix any visual inconsistencies. Sign off the design language.

#### Step 7.3: Accessibility Pass

- **Purpose**: Keyboard nav (tab between panels with 1–5 keys, arrow keys inside trees and lists), ARIA roles, focus rings, contrast ratios. The shadow DOM does not absolve us of a11y obligations to the host page's users.

#### Step 7.4: Visual Regression Baseline

- **Files to Create**: Playwright + `@playwright/test`'s `toHaveScreenshot` for Storybook stories, baseline images committed
- **Purpose**: Lock the visuals so the sidecar wiring work in Part B cannot quietly regress them.

#### Step 7.5: Cross-Panel Timeline Scrubber (Time-Travel Lite)

- **Files to Create**: `ui/src/components/TimelineScrubber.tsx`, `ui/src/stores/timelineStore.ts`, store-wide `getStateAt(timestamp)` accessors on every Zustand store.
- **Purpose**: A horizontal timeline anchored to the bottom of the sidebar, spanning the full retained event window. Drag the playhead and **every panel rewinds to that moment**: the variable inspector shows the values at time T, the log stream scrolls to entries from time T, the waterfall replays the streams that were active at time T, the type graph highlights types that were touched in that window. Releasing the playhead resumes live mode.
- **Architecture**: Every store gets a `getStateAt(timestamp)` derivation that walks its retained history. Panels read from `useTimelineState()` which returns either the live store value or the scrubbed value, depending on whether the scrubber is active. No replay engine — the data is already in the ring buffers; the scrubber is just an index into them.
- **Why this is the project's "killer feature"**: this is the single feature that operationalises observation-as-debugging (Decision 1). Without it, dev-suite is "live dashboard for dev-time data." With it, dev-suite becomes "rewind any moment in the last N seconds across every dimension of your system simultaneously" — something neither browser DevTools nor a step-debugger can do, because neither retains the whole-system history. This is the answer to "is this worth building?" — yes, *because of this step*.
- **Limitations to document**: limited to the retention window of each ring buffer (configurable). Will not show variables you didn't `watch()`. Will not let you edit-and-continue (read-only by design — for that, you still use the debugger).

---

### Phase 8: Rust Workspace + Protocol

End of Part A, start of Part B. The first Rust phase. Establishes the workspace, defines the protocol formally (matching the fixture shapes from Phase 1), and generates both codecs.

#### Step 8.1: Initialise Rust Workspace

- **Command**: `cargo init`, root `Cargo.toml` with workspace members `dev-suite-core`, `dev-suite-tui`, `dev-suite-protocol`, `dev-suite-waterfall`
- **Purpose**: Establish crate boundaries with minimal cross-dependencies. The TUI crate depends on the protocol crate but not on the OTLP receiver crate, so UI iteration doesn't trigger full rebuilds.
- **Concepts introduced**: cargo, crates, modules, workspace dependency resolution, `[workspace.dependencies]` for shared versions.

#### Step 8.2: Protocol Buffer Schema

- **Files to Create**: `proto/dev_suite.proto`
- **Purpose**: Define `VariableSnapshot`, `LogEntry`, `DataFlowEvent`, `TypeMetadata`, `TestResult`, `ConnectionHandshake`, `PanelSubscription`. Each message carries `timestamp`, `source_language`, `correlation_id`. **The field set must match the TypeScript fixture types from Phase 1.4** — if there is a mismatch, the fixture types win and the proto is corrected.
- **Design Decision**: protobuf over JSON for on-wire efficiency at thousands of events/second.

#### Step 8.3: Rust Codec (prost)

- **Files to Create**: `crates/dev-suite-protocol/src/messages.rs`, `crates/dev-suite-protocol/src/codec.rs`
- **Purpose**: `prost`-generated types + manual `From`/`Into` between protobuf types and internal domain types.
- **Concepts introduced**: `serde`, derive macros, generic functions, traits as capability contracts.

#### Step 8.4: TypeScript Codec (protobuf-ts)

- **Files to Create**: `ui/src/protocol/types.ts`, `ui/src/protocol/codec.ts`
- **Purpose**: Generated TypeScript types replace the hand-authored fixture types (the fixtures are then re-typed against the generated types — they must still type-check). Length-prefixed binary framing over the WebSocket.

#### Step 8.5: Rust Dev Tooling

- **Commands**: `cargo-watch`, `mold` linker (Linux), `clippy.toml` (pedantic), `rustfmt.toml`
- **Purpose**: Fast Rust iteration loop. `cargo watch -x check` runs alongside Vite.

---

### Phase 9: Core Sidecar Foundation

The Rust sidecar binary runs, accepts WebSocket connections, and fans out messages through an internal event bus. No collectors yet — just the skeleton.

#### Step 9.1: Axum HTTP Server + Static File Serving

- **Files to Create**: `crates/dev-suite-core/src/server/http.rs`, `config.rs`
- **Purpose**: Serve the React sidebar bundle on `:4400`, plus health and metrics endpoints. The bundle built in Phase 1.3 is now served from here.
- **Architecture Pattern**: Tower service layers (middleware stack). Axum's router with nested routes for `/`, `/health`, `/metrics`, `/ws`.
- **Concepts introduced**: `async fn`, `tokio`, axum extractors, `?` for error propagation.

#### Step 9.2: WebSocket Server + Event Bus

- **Files to Create**: `crates/dev-suite-core/src/server/websocket.rs`, `crates/dev-suite-core/src/bus/event_bus.rs`
- **Purpose**: Accept WebSocket connections from the sidebar (and later SDKs), route incoming messages to the event bus, broadcast events to all subscribers. **First implementation: hand-rolled MPMC over `Arc<Mutex<VecDeque<T>>>`**. Second pass: swap to `tokio::sync::broadcast` and document the upgrade in the commit message.
- **Architecture Pattern**: Publish-Subscribe (Observer pattern at the system level).
- **Concepts introduced**: `tokio::sync::broadcast`, slow-subscriber semantics, backpressure decisions (drop vs. buffer vs. block).

#### Step 9.3: CLI Entry Point + Process Wrapper Skeleton

- **Files to Create**: `crates/dev-suite-core/src/main.rs`, `crates/dev-suite-core/src/process/wrapper.rs`
- **Purpose**: `clap`-based CLI. Two modes: `dev-suite serve` (standalone sidecar) and `dev-suite run -- <command>` (wrap a target process, capture stdout/stderr). Wrapper uses `tokio::process::Command` with piped stdout/stderr.
- **Architecture Pattern**: Command pattern for subcommands. Strategy pattern for collection mode.

---

### Phase 10: Live Data Wiring (Replace Mocks)

The sidebar connects to the live sidecar, the connection-status indicator reflects reality, and the panels render real (still empty) data. No collectors yet, so the panels show their "awaiting data" states — but the path is now live.

#### Step 10.1: WebSocket Client with Reconnection

- **Files to Create**: `ui/src/protocol/client.ts`
- **Purpose**: Connect to `ws://localhost:4400/ws`. Handle binary protobuf messages. Auto-reconnect with exponential backoff + jitter. State machine: Disconnected → Connecting → Open → Closing. Buffer outgoing messages during outage (bounded queue, drop-oldest on overflow).
- **Architecture Pattern**: Adapter — translates WebSocket binary frames into typed store actions.

#### Step 10.2: Swap Stores from Fixtures to Live

- **Files to Modify**: every Zustand store created in Phases 3–6
- **Purpose**: Each store grows a `dataSource: 'fixture' | 'websocket'` toggle. Dev builds default to fixture, sidecar-connected runs default to websocket. The store API surface does not change — every panel keeps working untouched.

#### Step 10.3: Connection Status + Skeleton Crossfade

- **Purpose**: The mock connection-status hook from Phase 2.2 is now driven by the real WS client state. The waterfall skeleton mode crossfades to live data on connect.

---

### Phase 11: Sidecar Collectors

The five data stores in the sidecar that receive events and serve them out. This is the densest Rust phase — ownership, borrowing, lifetimes, and concurrency through real data structures.

#### Step 11.1: Variable Store

- **Files to Create**: `crates/dev-suite-core/src/collector/variable_store.rs`
- **Purpose**: Receives `VariableSnapshot` messages, stores latest N snapshots per variable path, supports diff detection. **First implementation: hand-rolled sharded `Mutex<HashMap<_, _>>`**. Second pass: adopt `dashmap`, measure contention, decide.
- **Data Structure**: HashMap for O(1) lookup by variable path, `VecDeque` ring buffer per key.
- **Concepts introduced**: ownership and borrowing under contention, `Arc<Mutex<_>>` vs `RwLock` vs `dashmap`.

#### Step 11.2: Log Buffer + stdout/stderr Capture

- **Files to Create**: `crates/dev-suite-core/src/collector/log_buffer.rs`, expand `process/wrapper.rs`
- **Purpose**: Ring buffer (default capacity 10,000) for `LogEntry` messages. Auxiliary `BTreeMap<LogLevel, Vec<usize>>` index for level-filtered iteration. When running in wrapper mode, parse common structured log formats (JSON lines, pino, Winston) into entries; raw text otherwise.

#### Step 11.3: Data Flow Graph

- **Files to Create**: `crates/dev-suite-core/src/collector/data_flow.rs`
- **Purpose**: Directed graph of data flow events. Nodes are endpoints / functions / channels; edges are HTTP requests, function calls, event emissions, DB queries. Sliding window of recent events for temporal visualisation.
- **Data Structure**: Adjacency list (`HashMap<NodeId, Vec<Edge>>`) + circular buffer of recent events.
- **Concepts introduced**: BFS/DFS, topological order, cycle detection, sliding-window edge sets.

#### Step 11.4: Type Registry

- **Files to Create**: `crates/dev-suite-core/src/collector/type_registry.rs`
- **Purpose**: Graph of type nodes (`name, kind, fields, methods, generic_params`). Edges represent field type reference, inheritance/implementation, generic instantiation. LRU cache for recently-queried lookups (hand-rolled doubly-linked list + HashMap first).

---

### Phase 12: TypeScript SDK

The first SDK, which makes dev-suite useful against a real instrumented target app for the first time.

#### Step 12.1: Variable Instrumentation

- **Files to Create**: `sdks/typescript/src/index.ts`, `client.ts`, `variables.ts`
- **Purpose**: `@dev-suite/sdk` package. `devSuite.watch('myVar', () => myVar)` registers a variable for periodic snapshot emission; `devSuite.snapshot({ key: value })` for one-shot captures. Connects via WebSocket, encodes protobuf, gracefully no-ops if the sidecar is unavailable.
- **Zero-overhead guarantee**: At import time, check `process.env.DEV_SUITE_ENABLED`. If unset, every export becomes a no-op function (tree-shakeable in production builds).

#### Step 12.2: Type Metadata Extraction

- **Files to Create**: `sdks/typescript/src/types.ts`
- **Purpose**: TypeScript Compiler API (`ts.createProgram`, `ts.TypeChecker`) extracts type information at dev time. Runs as a background process watching `.ts` files. Dev-time only.

---

### Phase 13: Tests Panel (Full Vertical)

The fifth panel — and the first one built end-to-end (collector + store + browser panel + SDK hook) after the sidecar exists. Validates the full pipeline.

#### Step 13.1: Test Aggregator (Sidecar)

- **Files to Create**: `crates/dev-suite-core/src/collector/test_aggregator.rs`
- **Purpose**: Receives `TestResult` messages from SDK reporter hooks. Stores latest results per test file/suite. Pass/fail/skip counts, durations, failure messages.

#### Step 13.2: Tests Panel — Browser

- **Files to Create**: `ui/src/panels/TestsPanel/index.tsx`, `TestResultTree.tsx`, `ui/src/stores/testStore.ts`, mock fixtures backfilled to `testFixtures.ts`
- **Purpose**: Tree view of suites and individual tests. Color-coded status (green=pass, red=fail, amber=skip, grey=pending). Click a failure for error + stack trace. Live-updates.

#### Step 13.3: SDK Reporter Hooks

- **Files to Create**: `sdks/typescript/src/testing.ts`
- **Purpose**: Vitest reporter plugin and Jest reporter that emit `TestResult` messages as tests complete. Auto-detected when `vitest` or `jest` is in `node_modules`.

---

### Phase 14: TUI

The terminal frontend reaches parity with the browser sidebar. Same data, rendered for terminals.

#### Step 14.1: ratatui Application Scaffold

- **Files to Create**: `crates/dev-suite-tui/src/app.rs`, `events.rs`, `ui/layout.rs`
- **Purpose**: Immediate-mode render loop with crossterm. Tab navigation (1–5 keys), status bar showing connection state and event rate.
- **Architecture Pattern**: Model-View-Update.

#### Step 14.2: Sidecar Integration (In-Process)

- **Files to Modify**: `crates/dev-suite-core/src/main.rs`
- **Purpose**: `dev-suite serve --tui` spawns the TUI on the main thread and runs the server on background tokio tasks. The TUI subscribes directly to the event bus channels — no WebSocket needed. Same-process rendering is the performance advantage.

#### Step 14.3: Five Panel Implementations

- **Files to Create**: `crates/dev-suite-tui/src/ui/variables_panel.rs`, `logs_panel.rs`, `dataflow_panel.rs`, `types_panel.rs`, `tests_panel.rs`
- **Purpose**: Terminal renderings of the same data the browser panels show. Variables: ratatui `Table`. Logs: scrollable list + level-shortcut keys (`e`, `w`, `a`), tail mode (`t`), search (`/`). Tests: table + summary line. Types: ASCII-art / tree view.

#### Step 14.4: Matrix Waterfall (TUI)

- **Files to Create**: `crates/dev-suite-tui/src/ui/dataflow_panel.rs`, expand `crates/dev-suite-waterfall/src/`
- **Purpose**: Terminal-native waterfall using ratatui's `Buffer` directly. Unicode block elements + ASCII. Colour via ANSI 256 / truecolor. The `dev-suite-waterfall` crate provides the shared logic — given a grid (rows × cols), it computes which character goes where at time T. Designed `no_std`-compatible to also compile to WASM (future ratzilla integration).
- **Concepts introduced**: arena allocation, `no_std`, a crate compiled to both native and WASM.

---

### Phase 15: OTLP Integration

Accept standard OpenTelemetry data, making dev-suite compatible with any OTel-instrumented application.

#### Step 15.1: OTLP gRPC Receiver

- **Files to Create**: `crates/dev-suite-core/src/server/otlp.rs`
- **Purpose**: Implement OTLP gRPC endpoints (`TraceService/Export`, `LogsService/Export`, `MetricsService/Export`) using `tonic`. Listen on `:4317` (gRPC) and `:4318` (HTTP/protobuf) per OTLP convention.
- **Concepts introduced**: trait objects, dynamic dispatch, gRPC streaming, `Pin` in practice.

#### Step 15.2: Semantic Convention Mapping

- **Purpose**: Map OTLP semantic conventions (`http.method`, `db.system`, `rpc.service`) to dev-suite's data flow graph. An HTTP span with `http.method=GET` and `http.route=/api/users` becomes a node labelled "GET /api/users". A database span becomes "PostgreSQL: SELECT". OTLP data is immediately useful in the waterfall.

---

### Phase 16: Additional Language SDKs

Extend beyond TypeScript to support the polyglot workflow.

#### Step 16.1: Python SDK

- **Files to Create**: `sdks/python/src/dev_suite/`
- **Purpose**: `pip install dev-suite-sdk`. Provides `dev_suite.watch()`, `dev_suite.snapshot()`, pytest plugin for test result emission, `logging.Handler` subclass. Uses `threading.Thread` for async WebSocket communication.

#### Step 16.2: Rust SDK

- **Files to Create**: `sdks/rust/src/`
- **Purpose**: `dev-suite-sdk` crate. Procedural macros: `#[dev_suite::watch]` on a variable, `#[dev_suite::instrument]` on functions (emits entry/exit data flow events). Integrates with `tracing` as a subscriber layer.
- **Concepts introduced**: procedural macros, `tracing` integration, `Send`/`Sync` bounds on user-supplied closures.

#### Step 16.3: Go, Java, C#, C++ SDK Stubs

- **Purpose**: Minimal stubs demonstrating the protocol. Java/C# wrap the existing OTLP SDKs with a thin dev-suite extension. Go uses `go.opentelemetry.io/otel`. C++ wraps the OpenTelemetry C++ SDK.

---

### Phase 17: Plugin System & METAL Dashboard

Prove the extensibility architecture by integrating the METAL dashboard as a plugin.

#### Step 17.1: Plugin API

- **Files to Create**: `docs/PLUGIN_API.md`, plugin loader in sidecar
- **Purpose**: A plugin registers a panel name, a data subscription filter (which event types it cares about), and either a React component (browser) or a ratatui widget (TUI). Loaded from `~/.dev-suite/plugins/` or via config.

#### Step 17.2: METAL Dashboard Plugin

- **Purpose**: Renders the METAL (Metrics, Events, Traces, Alerts, Logs) dashboard inside dev-suite. Subscribes to all event types. Reuses components from the existing METAL project. The TUI panel provides a compact summary view.

---

### Phase 18: Testing Strategy (Full Pyramid)

#### Testing Philosophy

- **Approach**: Pragmatic — tests alongside code, not strictly test-first. Emphasis on testing boundaries (protocol codec, WebSocket framing, event bus dispatch) over internal implementation details. (Vitest unit tests were already in place from Phase 1 onward; Phase 7.4 added Playwright visual baselines. This phase fills the remaining levels.)
- **Coverage Target**: 80% unit (Rust + TypeScript), 60% integration.

#### Step 18.1: Rust Unit Tests

- **Framework**: Built-in `#[test]` + `tokio::test`
- **Focus**: Protocol codec round-trip (`proptest`), event bus delivery guarantees, ring buffer behaviour at capacity, OTLP message mapping.

#### Step 18.2: Integration Tests

- **Framework**: Rust integration tests spawning the full sidecar; TypeScript tests with real WebSocket connections
- **Focus**: SDK → sidecar → browser sidebar end-to-end. OTLP ingestion → event bus → panel rendering.

#### Step 18.3: E2E Tests

- **Framework**: Playwright (extended from the Phase 7.4 visual baselines)
- **Focus**: Sidebar injection into a real test page, panel tab switching, variable update rendering, log filtering, waterfall animation under load.

#### Step 18.4: Performance Benchmarks

- **Framework**: `criterion` (Rust), custom WebSocket throughput harness
- **Focus**: Events/sec throughput, sidecar memory under sustained load, TUI frame time, WebGL waterfall FPS.

---

### Phase 19: DevOps, Automation & Deployment

#### Step 19.1: Containerization

- **Files to Create**: `Dockerfile` (multi-stage Rust builder → scratch/distroless), `docker-compose.yml`
- **Multi-stage Build**: Stage 1 — `rust:1.XX-bookworm` compiles sidecar. Stage 2 — `node:22-alpine` builds React sidebar. Stage 3 — `scratch` / `gcr.io/distroless/static` with just the binary + static assets.
- **Security**: Trivy scan in pipeline, non-root user, no shell in final image.

#### Step 19.2: Gitea + Jenkins Local Setup

- **Command**: `docker-compose up` with Gitea and Jenkins containers, initial repo creation, Jenkinsfile skeleton
- **Purpose**: CI/CD infrastructure running locally. Gitea webhook triggers Jenkins on push. (Deferred from project setup — the prototype phases ran without remote CI; this is the point where the project earns it.)

#### Step 19.3: Jenkins CI/CD Pipeline

- **Files to Create**: `Jenkinsfile`
- **Stages**: Checkout → Lint (clippy + ESLint) → Test (cargo test + vitest) → Security scan (cargo-audit + npm audit + Trivy) → Build (cargo build --release + npm run build) → Container build → Push to registry.
- **Artifacts**: Linux amd64 binary, container image, npm package tarball.

#### Step 19.4: Infrastructure as Code (AWS Demo)

- **Tool**: Terraform
- **Files to Create**: `terraform/main.tf`, `variables.tf`, `outputs.tf`
- **Purpose**: Optional — provision an EC2 instance running the demo (sidecar + example app + Gitea + Jenkins). Remote state in S3 + DynamoDB locking.

#### Step 19.5: Observability for the Tool Itself

- **Structured logging**: `tracing` crate with `tracing-subscriber` JSON formatter.
- **Metrics**: Prometheus endpoint at `:4400/metrics` (event counters, buffer utilisation, connection count, frame render time).
- **Dogfooding**: run a second dev-suite instance pointing at the first.

#### Step 19.6: Security Scanning Automation

- **SAST**: cargo-clippy (pedantic), ESLint with security rules.
- **Dependency scanning**: cargo-audit, cargo-deny (license + advisory), npm audit.
- **Container scanning**: Trivy in pipeline.
- **Secret scanning**: gitleaks as pre-commit hook + pipeline stage.
- **IaC scanning**: tfsec for Terraform.

---

### Phase 20: Documentation & Finalization

#### Step 20.1: README & Setup Guide

- **Files**: `README.md` with quickstart, architecture diagram, screenshots/GIFs of both frontends, installation.

#### Step 20.2: Architecture Documentation

- **Files**: `docs/ARCHITECTURE.md` — component descriptions, data-flow diagrams, decision log referencing the Key Technical Decisions section.

#### Step 20.3: Protocol Specification

- **Files**: `docs/PROTOCOL.md` — message reference, connection lifecycle, versioning.

#### Step 20.4: SDK Guide

- **Files**: `docs/SDK_GUIDE.md` — how to instrument an app, per-language quickstarts, zero-overhead guarantees.

#### Step 20.5: Plugin API Documentation

- **Files**: `docs/PLUGIN_API.md` — how to build a plugin; METAL as the reference implementation.

---

## Dependency List

### Rust Production Dependencies

| Crate | Purpose |
|---|---|
| tokio | Async runtime (full features: rt-multi-thread, net, sync, process, signal) |
| axum | HTTP server, WebSocket upgrade, static file serving |
| tonic | gRPC server for OTLP receiver |
| prost | Protocol Buffer code generation for Rust |
| ratatui | TUI rendering framework |
| crossterm | Terminal backend for ratatui |
| serde | Serialization framework (JSON fallback for non-protobuf messages) |
| serde_json | JSON serialization |
| clap | CLI argument parsing |
| tracing | Structured logging and instrumentation |
| tracing-subscriber | Log formatting and filtering |
| uuid | Correlation ID generation |
| chrono | Timestamp handling |
| dashmap | Concurrent HashMap for variable store (lock-free reads) |
| bytes | Efficient byte buffer management |

### Rust Development Dependencies

| Crate | Purpose |
|---|---|
| proptest | Property-based testing for codec round-trips |
| criterion | Benchmarking (throughput, frame time) |
| tokio-test | Async test utilities |
| cargo-audit | Dependency vulnerability scanning |
| cargo-deny | License and advisory checking |

### TypeScript Production Dependencies (ui/)

| Package | Purpose |
|---|---|
| react, react-dom | UI framework |
| zustand | State management (minimal, works well with streaming data) |
| protobufjs | Protocol Buffer runtime for TypeScript |
| react-window | Virtualized list rendering (log panel) |
| three | WebGL rendering (waterfall visualizer) |
| d3-force | Force-directed graph layout (type graph) |
| tailwindcss | Utility-first CSS (inside shadow DOM) |

### TypeScript Development Dependencies (ui/)

| Package | Purpose |
|---|---|
| vite | Build tool and dev server |
| typescript | Type checking |
| vitest | Unit testing |
| @testing-library/react | Component testing utilities |
| playwright | E2E testing |
| eslint | Linting |
| prettier | Code formatting |
| protobuf-ts | TypeScript code generation from proto files |

---

## Environment Variables

| Variable | Description | Required | Default |
|---|---|---|---|
| DEV_SUITE_PORT | Primary port for HTTP/WebSocket server | No | 4400 |
| DEV_SUITE_OTLP_GRPC_PORT | OTLP gRPC receiver port | No | 4317 |
| DEV_SUITE_OTLP_HTTP_PORT | OTLP HTTP receiver port | No | 4318 |
| DEV_SUITE_LOG_BUFFER_SIZE | Max log entries in ring buffer | No | 10000 |
| DEV_SUITE_VAR_HISTORY_SIZE | Max snapshots per variable path | No | 100 |
| DEV_SUITE_LOG_LEVEL | Sidecar internal log level | No | info |
| DEV_SUITE_ENABLED | SDK activation flag (set in target app) | No | false |
| DEV_SUITE_TUI | Launch TUI on startup | No | false |

---

## Success Criteria

- [ ] Sidecar binary compiles and runs on Linux, macOS, Windows
- [ ] TUI renders all five panels with real data from an instrumented TypeScript app
- [ ] Browser sidebar injects cleanly into a Next.js app without breaking it
- [ ] Matrix waterfall renders live data at 60fps in browser, 30fps in terminal
- [ ] OTLP ingestion works with a standard OTel-instrumented Python Flask app
- [ ] TypeScript SDK adds < 1ms overhead per instrumented operation
- [ ] All tests passing: unit 80%+, integration 60%+
- [ ] Jenkins pipeline: lint → test → security scan → build → container → green
- [ ] Gitea webhook triggers Jenkins on push
- [ ] Container image < 50MB (distroless + static binary + UI assets)
- [ ] `cargo-audit` and `npm audit` clean
- [ ] Trivy scan clean (no critical/high CVEs)
- [ ] README includes architecture diagram, quickstart, and screenshots/GIFs
- [ ] METAL dashboard plugin loads and renders in both frontends
- [ ] Documentation covers architecture, protocol, SDK guide, and plugin API