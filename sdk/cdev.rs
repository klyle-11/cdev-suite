//! cdev Rust SDK — one file, std only, no dependencies.
//!
//! Copy into your crate as `src/cdev.rs`, add `#[macro_use] mod cdev;` to main.rs/lib.rs, then:
//!
//! ```ignore
//! let total = cdev_w!(price * qty);            // records "price * qty" = 42 : i32, returns the value
//! cdev_w!("head", &head);                      // explicit label; any `T: Debug`
//! fn solve(n: usize, grid: &Vec<Vec<u8>>) -> usize {
//!     cdev_trace!(n, grid);                    // args + duration + caller, recorded when the scope ends
//!     cdev_log!("n is {}", n);
//!     0
//! }
//! ```
//!
//! Values are sent as their `{:?}` text; the sidecar parses it back into
//! structure, so `Option<Box<ListNode>>` draws as a linked list and
//! `Vec<Vec<i32>>` as a grid. Events are batched on a background thread and
//! flushed when `main` returns. Set CDEV_OFF=1 to disable.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Client {
    addr: String,
    svc: String,
    off: bool,
    queue: Mutex<Vec<String>>,
}

fn client() -> &'static Client {
    static C: OnceLock<Client> = OnceLock::new();
    C.get_or_init(|| {
        let url = std::env::var("CDEV_URL").unwrap_or_else(|_| "http://127.0.0.1:4400".into());
        let addr = url.split("://").last().unwrap_or("127.0.0.1:4400").split('/').next().unwrap_or("127.0.0.1:4400").replace("localhost", "127.0.0.1");
        let svc = std::env::var("CDEV_SERVICE").unwrap_or_else(|_| {
            std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_else(|| "rust".into())
        });
        let off = std::env::var_os("CDEV_OFF").is_some();
        if !off {
            std::thread::spawn(|| loop {
                std::thread::sleep(Duration::from_millis(50));
                flush();
            });
            // flush when main returns (std calls libc `exit`, which runs atexit handlers)
            extern "C" {
                fn atexit(cb: extern "C" fn()) -> i32;
            }
            extern "C" fn at_exit() {
                flush();
            }
            unsafe {
                atexit(at_exit);
            }
        }
        Client { addr, svc, off, queue: Mutex::new(Vec::new()) }
    })
}

/// JSON string literal.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// `alloc::vec::Vec<core::option::Option<i32>>` → `Vec<Option<i32>>`
pub fn short_type(t: &str) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ':' && chars.peek() == Some(&':') {
            chars.next();
            word.clear(); // drop the module path segment
            continue;
        }
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            out.push_str(&word);
            word.clear();
            out.push(c);
        }
    }
    out.push_str(&word);
    out
}

/// Queue one event; `fields` is the JSON body without braces.
pub fn send(fields: String) {
    let c = client();
    if c.off {
        return;
    }
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);
    let ev = format!("{{\"ts\":{ts},\"svc\":{},\"lang\":\"rust\",\"pid\":{},{fields}}}", esc(&c.svc), std::process::id());
    let mut q = c.queue.lock().unwrap_or_else(|e| e.into_inner());
    q.push(ev);
    if q.len() > 5000 {
        q.remove(0);
    }
}

/// Send everything queued now (blocking). Called automatically; call it
/// yourself before `std::process::exit`.
pub fn flush() {
    let c = client();
    let batch: Vec<String> = std::mem::take(&mut *c.queue.lock().unwrap_or_else(|e| e.into_inner()));
    if batch.is_empty() {
        return;
    }
    let body = format!("[{}]", batch.join(","));
    if let Ok(mut s) = TcpStream::connect(&c.addr) {
        let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
        let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
        let req = format!(
            "POST /ingest HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            c.addr,
            body.len()
        );
        if s.write_all(req.as_bytes()).is_ok() {
            let mut sink = [0u8; 256];
            while matches!(s.read(&mut sink), Ok(n) if n > 0) {}
        }
    }
}

thread_local! {
    static STACK: RefCell<Vec<String>> = RefCell::new(Vec::new());
    static LAST: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

fn caller() -> String {
    STACK.with(|s| s.borrow().last().map(|c| esc(c)).unwrap_or_else(|| "null".into()))
}

// ---------------------------------------------------------------- memory

/// Heap buffer / pointee details found by `probe` for the types that have one.
pub struct Mem {
    pub ptr: Option<usize>,
    pub target_size: usize,
    pub heap: Option<(usize, usize, usize, usize)>, // (addr, len, cap, elem size)
    pub refs: Option<usize>,
}

/// Picks type-specific memory info on stable Rust ("autoref specialisation"):
/// `(&probe::P(&v)).mem()` resolves to the most specific impl, falling back to none.
pub mod probe {
    use super::Mem;
    use std::mem::size_of;
    use std::rc::Rc;
    use std::sync::Arc;

    pub struct P<'a, T: ?Sized>(pub &'a T);

    fn buf(addr: usize, len: usize, cap: usize, elem: usize) -> Option<Mem> {
        Some(Mem { ptr: None, target_size: 0, heap: Some((addr, len, cap, elem)), refs: None })
    }
    fn to(ptr: usize, size: usize, refs: Option<usize>) -> Option<Mem> {
        Some(Mem { ptr: Some(ptr), target_size: size, heap: None, refs })
    }

    pub trait ViaVec { fn mem(&self) -> Option<Mem>; }
    impl<T> ViaVec for P<'_, Vec<T>> { fn mem(&self) -> Option<Mem> { buf(self.0.as_ptr() as usize, self.0.len(), self.0.capacity(), size_of::<T>()) } }
    impl<T> ViaVec for P<'_, &Vec<T>> { fn mem(&self) -> Option<Mem> { buf(self.0.as_ptr() as usize, self.0.len(), self.0.capacity(), size_of::<T>()) } }
    pub trait ViaString { fn mem(&self) -> Option<Mem>; }
    impl ViaString for P<'_, String> { fn mem(&self) -> Option<Mem> { buf(self.0.as_ptr() as usize, self.0.len(), self.0.capacity(), 1) } }
    impl ViaString for P<'_, &String> { fn mem(&self) -> Option<Mem> { buf(self.0.as_ptr() as usize, self.0.len(), self.0.capacity(), 1) } }
    pub trait ViaBox { fn mem(&self) -> Option<Mem>; }
    impl<T> ViaBox for P<'_, Box<T>> { fn mem(&self) -> Option<Mem> { to(&**self.0 as *const T as usize, size_of::<T>(), None) } }
    impl<T> ViaBox for P<'_, Option<Box<T>>> {
        fn mem(&self) -> Option<Mem> { self.0.as_ref().and_then(|b| to(&**b as *const T as usize, size_of::<T>(), None)) }
    }
    pub trait ViaRc { fn mem(&self) -> Option<Mem>; }
    impl<T> ViaRc for P<'_, Rc<T>> { fn mem(&self) -> Option<Mem> { to(Rc::as_ptr(self.0) as usize, size_of::<T>(), Some(Rc::strong_count(self.0))) } }
    pub trait ViaArc { fn mem(&self) -> Option<Mem>; }
    impl<T> ViaArc for P<'_, Arc<T>> { fn mem(&self) -> Option<Mem> { to(Arc::as_ptr(self.0) as usize, size_of::<T>(), Some(Arc::strong_count(self.0))) } }

    impl<T> ViaBox for P<'_, &Box<T>> { fn mem(&self) -> Option<Mem> { to(&***self.0 as *const T as usize, size_of::<T>(), None) } }
    impl<T> ViaBox for P<'_, &Option<Box<T>>> {
        fn mem(&self) -> Option<Mem> { self.0.as_ref().and_then(|b| to(&**b as *const T as usize, size_of::<T>(), None)) }
    }
    impl<T> ViaRc for P<'_, &Rc<T>> { fn mem(&self) -> Option<Mem> { to(Rc::as_ptr(*self.0) as usize, size_of::<T>(), Some(Rc::strong_count(*self.0))) } }
    impl<T> ViaArc for P<'_, &Arc<T>> { fn mem(&self) -> Option<Mem> { to(Arc::as_ptr(*self.0) as usize, size_of::<T>(), Some(Arc::strong_count(*self.0))) } }

    pub trait Fallback { fn mem(&self) -> Option<Mem> { None } }
    impl<T: ?Sized> Fallback for &P<'_, T> {}

    /// For references, report where the referent lives (`cdev_w!(&x)` shows x's real address).
    pub trait RefTarget { fn target(&self) -> Option<(usize, usize)>; }
    impl<U> RefTarget for P<'_, &U> { fn target(&self) -> Option<(usize, usize)> { Some((*self.0 as *const U as usize, size_of::<U>())) } }
    impl<U> RefTarget for P<'_, &mut U> { fn target(&self) -> Option<(usize, usize)> { Some((&**self.0 as *const U as usize, size_of::<U>())) } }
    pub trait NoTarget { fn target(&self) -> Option<(usize, usize)> { None } }
    impl<T: ?Sized> NoTarget for &P<'_, T> {}
}

fn hex(a: usize) -> String {
    format!("\"0x{a:x}\"")
}

static STATIC_MARKER: u8 = 0;

/// stack / heap / static, by distance from the current stack pointer and a static.
pub fn region_of(a: usize) -> &'static str {
    let probe = 0u8;
    let sp = &probe as *const u8 as usize;
    if a + 65536 >= sp && a < sp + (8 << 20) {
        return "stack";
    }
    let st = &STATIC_MARKER as *const u8 as usize;
    if a + (64 << 20) > st && a < st + (64 << 20) {
        return "static";
    }
    "heap"
}

/// JSON for the `mem` field. `addr`/`size` describe where the value itself lives.
pub fn mem_json(addr: usize, size: usize, extra: Option<Mem>) -> String {
    let mut o = format!("{{\"addr\":{},\"size\":{size},\"region\":\"{}\"", hex(addr), region_of(addr));
    if let Some(m) = extra {
        if let Some(p) = m.ptr {
            o += &format!(",\"ptr\":{},\"target_size\":{},\"target_region\":\"{}\"", hex(p), m.target_size, region_of(p));
        }
        if let Some(r) = m.refs {
            o += &format!(",\"refs\":{r}");
        }
        if let Some((a, len, cap, elem)) = m.heap {
            // String/Vec with capacity 0 point at a dangling (non-allocated) address
            let region = if cap == 0 { "none" } else { region_of(a) };
            o += &format!(",\"heap\":{{\"addr\":{},\"len\":{len},\"cap\":{cap},\"elem\":{elem},\"region\":\"{region}\"}}", hex(a));
        }
    }
    o + "}"
}

/// Used by `cdev_w!`. Skips values identical to the last one under `name`.
pub fn watch<T: std::fmt::Debug + ?Sized>(name: &str, v: &T, file: &str, line: u32) {
    watch_mem(name, v, file, line, String::new());
}

pub fn watch_mem<T: std::fmt::Debug + ?Sized>(name: &str, v: &T, file: &str, line: u32, mem: String) {
    if client().off {
        return;
    }
    let dbg = format!("{v:?}");
    let key = format!("{file}:{name}");
    let state = format!("{dbg}{mem}"); // a move or reallocation counts as a change
    let same = LAST.with(|l| {
        let mut l = l.borrow_mut();
        if l.get(&key) == Some(&state) {
            true
        } else {
            l.insert(key, state);
            false
        }
    });
    if same {
        return;
    }
    let mem = if mem.is_empty() { String::new() } else { format!(",\"mem\":{mem}") };
    send(format!(
        "\"kind\":\"watch\",\"name\":{},\"vdebug\":{},\"t\":{}{mem},\"loc\":{},\"caller\":{}",
        esc(name),
        esc(&dbg),
        esc(&short_type(std::any::type_name::<T>())),
        esc(&format!("{file}:{line}")),
        caller()
    ));
}

pub fn log(text: String, file: &str, line: u32) {
    send(format!("\"kind\":\"log\",\"level\":\"info\",\"text\":{},\"loc\":{},\"caller\":{}", esc(&text), esc(&format!("{file}:{line}")), caller()));
}

/// Scope guard created by `cdev_trace!`: one `call` event when it drops.
pub struct Scope {
    name: String,
    loc: String,
    caller: String,
    params: Vec<&'static str>,
    args: Vec<String>,
    types: Vec<String>,
    t0: Instant,
}

impl Scope {
    pub fn new(func: &str, file: &str, line: u32, names: &'static str, args: Vec<(String, String)>) -> Scope {
        // `crate::module::solve::{{closure}}::f` → `solve`
        let name = func.trim_end_matches("::f").trim_end_matches("::{{closure}}").rsplit("::").next().unwrap_or(func).to_string();
        let caller = caller();
        STACK.with(|s| s.borrow_mut().push(name.clone()));
        let params: Vec<&'static str> = if names.trim().is_empty() { vec![] } else { names.split(',').map(str::trim).collect() };
        let (args, types) = args.into_iter().unzip();
        Scope { name, loc: format!("{file}:{line}"), caller, params, args, types, t0: Instant::now() }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        STACK.with(|s| {
            s.borrow_mut().pop();
        });
        if client().off {
            return;
        }
        let list = |v: &[String]| format!("[{}]", v.iter().map(|x| esc(x)).collect::<Vec<_>>().join(","));
        let params: Vec<String> = self.params.iter().map(|p| p.to_string()).collect();
        let err = if std::thread::panicking() { ",\"err\":\"panic\"" } else { "" };
        send(format!(
            "\"kind\":\"call\",\"name\":{},\"params\":{},\"argsDebug\":{},\"argTypes\":{},\"ms\":{:.3},\"loc\":{},\"caller\":{}{err}",
            esc(&self.name),
            list(&params),
            list(&self.args),
            list(&self.types),
            self.t0.elapsed().as_secs_f64() * 1000.0,
            esc(&self.loc),
            self.caller
        ));
    }
}

pub fn arg<T: std::fmt::Debug + ?Sized>(v: &T) -> (String, String) {
    (format!("{v:?}"), short_type(std::any::type_name::<T>()))
}

/// Record a value (Debug text + type + location) and return it.
/// `cdev_w!(expr)` labels it with the expression text; `cdev_w!("label", expr)` sets one.
/// Pass a reference (`cdev_w!(&x)`) to see where `x` itself lives; passing `x` by
/// value moves it, so the address shown is the macro's temporary.
#[macro_export]
macro_rules! cdev_w {
    ($name:literal, $e:expr) => {{
        let v = $e;
        $crate::cdev_w!(@send $name, v);
        v
    }};
    ($e:expr) => {{
        let v = $e;
        $crate::cdev_w!(@send stringify!($e), v);
        v
    }};
    (@send $name:expr, $v:ident) => {{
        #[allow(unused_imports)]
        use $crate::cdev::probe::{Fallback, NoTarget, RefTarget, ViaArc, ViaBox, ViaRc, ViaString, ViaVec};
        let probe = $crate::cdev::probe::P(&$v);
        let (addr, size) = (&probe).target().unwrap_or((&$v as *const _ as usize, ::std::mem::size_of_val(&$v)));
        let mem = $crate::cdev::mem_json(addr, size, (&probe).mem());
        $crate::cdev::watch_mem($name, &$v, file!(), line!(), mem);
    }};
}

/// Record this function's arguments and duration when the scope ends.
#[macro_export]
macro_rules! cdev_trace {
    ($($arg:expr),* $(,)?) => {
        let _cdev_scope = {
            fn f() {}
            fn name_of<T>(_: T) -> &'static str { ::std::any::type_name::<T>() }
            $crate::cdev::Scope::new(name_of(f), file!(), line!(), stringify!($($arg),*), vec![$($crate::cdev::arg(&$arg)),*])
        };
    };
}

/// `format!`-style log line with source location.
#[macro_export]
macro_rules! cdev_log {
    ($($t:tt)*) => { $crate::cdev::log(format!($($t)*), file!(), line!()) };
}
