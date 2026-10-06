//! `cdev run -- <cmd>`: spawn the target with SDK hooks injected and turn its
//! stdout/stderr into log events.

use crate::event::{now_ms, Event};
use crate::store::Store;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::Instant;

pub const NODE_SDK: &str = include_str!("../sdk/cdev-node.js");
pub const PY_SDK: &str = include_str!("../sdk/cdev.py");
pub const CPP_SDK: &str = include_str!("../sdk/cdev.hpp");
pub const TS_DEFS: &str = include_str!("../sdk/cdev.d.ts");
pub const RUST_SDK: &str = include_str!("../sdk/cdev.rs");
pub const GO_SDK: &str = include_str!("../sdk/cdev.go");
pub const PHP_SDK: &str = include_str!("../sdk/cdev.php");
pub const BUN_BUILD: &str = include_str!("../sdk/cdev-bun-build.js");
const GO_MOD: &str = "module cdev\n\ngo 1.21\n";
const SITECUSTOMIZE: &str = "try:\n    import cdev\n    cdev._auto()\nexcept Exception:\n    pass\n";

#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// Separator for PATH-like env vars (PYTHONPATH, PHP_INI_SCAN_DIR).
pub const PATH_SEP: char = if cfg!(windows) { ';' } else { ':' };

/// Process-group id of the wrapped app, so quitting can stop it too.
pub static APP_PGID: AtomicI32 = AtomicI32::new(0);

/// Write every SDK file into `dir`. `with_hooks` adds the Python
/// sitecustomize auto-hook (only wanted in cdev's own temp dir).
pub fn write_sdk(dir: &Path, with_hooks: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("cdev-node.js"), NODE_SDK)?;
    std::fs::write(dir.join("cdev.py"), PY_SDK)?;
    std::fs::write(dir.join("cdev.js"), crate::server::BROWSER_JS)?;
    std::fs::write(dir.join("cdev.hpp"), CPP_SDK)?;
    std::fs::write(dir.join("cdev.d.ts"), TS_DEFS)?;
    std::fs::write(dir.join("cdev.rs"), RUST_SDK)?;
    std::fs::write(dir.join("cdev.php"), PHP_SDK)?;
    // Go: a tiny module, used via `require cdev v0.0.0` + `replace cdev => ./cdev`
    std::fs::create_dir_all(dir.join("cdev-go"))?;
    std::fs::write(dir.join("cdev-go").join("cdev.go"), GO_SDK)?;
    std::fs::write(dir.join("cdev-go").join("go.mod"), GO_MOD)?;
    if with_hooks {
        std::fs::write(dir.join("cdev-bun-build.js"), BUN_BUILD)?;
        std::fs::write(dir.join("sitecustomize.py"), SITECUSTOMIZE)?;
        // PHP reads extra .ini files from PHP_INI_SCAN_DIR; this one preloads the SDK
        std::fs::create_dir_all(dir.join("php-ini"))?;
        std::fs::write(dir.join("php-ini").join("cdev.ini"), format!("auto_prepend_file=\"{}\"\n", dir.join("cdev.php").display().to_string().replace('\\', "/")))?;
    }
    Ok(())
}

/// Find `bin` on PATH (on Windows also trying PATHEXT: `npm` → `npm.cmd`).
pub fn which(bin: &str) -> Option<PathBuf> {
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';').filter(|e| !e.is_empty()).map(str::to_lowercase).collect()
    } else {
        Vec::new()
    };
    let found = |p: PathBuf| -> Option<PathBuf> {
        if p.is_file() && (!cfg!(windows) || p.extension().is_some()) {
            return Some(p);
        }
        exts.iter().map(|e| PathBuf::from(format!("{}{e}", p.display()))).find(|c| c.is_file())
    };
    if bin.contains('/') || bin.contains('\\') {
        return found(PathBuf::from(bin));
    }
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|d| found(d.join(bin)))
}

/// Program to spawn for `bin`: Windows only auto-resolves `.exe`, so shims
/// like `npm.cmd` / `npx.cmd` need their full path.
pub fn program(bin: &str) -> PathBuf {
    if cfg!(windows) {
        which(bin).unwrap_or_else(|| PathBuf::from(bin))
    } else {
        PathBuf::from(bin)
    }
}

/// Temp dir holding the SDK files injected into wrapped apps.
pub fn sdk_dir() -> PathBuf {
    std::env::temp_dir().join(format!("cdev-sdk-{}", env!("CARGO_PKG_VERSION")))
}

pub fn service_name() -> String {
    std::env::var("CDEV_SERVICE").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| {
        std::env::current_dir().ok()
            .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "app".into())
    })
}

pub async fn run(store: Arc<Store>, cmd: Vec<String>, port: u16) -> i32 {
    run_as(store, cmd, port, None).await
}

/// Like `run`, with a short label shown instead of the full command line.
pub async fn run_as(store: Arc<Store>, cmd: Vec<String>, port: u16, label: Option<String>) -> i32 {
    let svc = service_name();
    let sdk_dir = sdk_dir();
    if let Err(e) = write_sdk(&sdk_dir, true) {
        store.push(Event::new("error", "cdev").with("msg", format!("could not write SDK files: {e}")));
    }
    let node_req = sdk_dir.join("cdev-node.js");
    // NODE_OPTIONS treats `\` inside quotes as an escape; Node accepts `/` on Windows too
    let node_opts = format!(
        "{} --require \"{}\"",
        std::env::var("NODE_OPTIONS").unwrap_or_default(),
        node_req.display().to_string().replace('\\', "/")
    );
    let mut py_dirs = vec![sdk_dir.clone()];
    if let Some(p) = std::env::var_os("PYTHONPATH") {
        py_dirs.extend(std::env::split_paths(&p));
    }
    let py_path = std::env::join_paths(py_dirs).unwrap_or_else(|_| sdk_dir.clone().into_os_string());
    // leading separator appends to PHP's default scan dirs instead of replacing them
    let php_scan = format!("{PATH_SEP}{}", sdk_dir.join("php-ini").display());

    let mut c = Command::new(program(&cmd[0]));
    c.args(&cmd[1..])
        .env("CDEV_URL", format!("http://127.0.0.1:{port}"))
        .env("CDEV_SERVICE", &svc)
        .env("CDEV_WRAPPED", "1")
        .env("CDEV_AUTO", "1")
        .env("CDEV_BIN", std::env::current_exe().unwrap_or_default())
        .env("NODE_OPTIONS", node_opts.trim())
        .env("PYTHONPATH", py_path)
        .env("PYTHONUNBUFFERED", "1")
        .env("PHP_INI_SCAN_DIR", php_scan)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true); // tokio API name: stop the app if cdev exits unexpectedly
    // own process group, so stop_app reaches npm → node → workers
    #[cfg(unix)]
    c.process_group(0);
    #[cfg(windows)]
    c.creation_flags(CREATE_NEW_PROCESS_GROUP);

    let shown = label.unwrap_or_else(|| cmd.join(" "));
    let mut app = match c.spawn() {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("failed to start `{shown}`: {e}");
            store.push(Event::new("error", "cdev").with("msg", msg.clone()));
            store.set_app_status(msg);
            return 127;
        }
    };
    if let Some(pid) = app.id() {
        APP_PGID.store(pid as i32, Ordering::SeqCst);
    }
    store.set_app_status(format!("running: {shown}"));

    let (tx, rx) = mpsc::unbounded_channel::<(Instant, f64, &'static str, String)>();
    if let Some(o) = app.stdout.take() {
        tokio::spawn(read_lines(o, "stdout", tx.clone()));
    }
    if let Some(e) = app.stderr.take() {
        tokio::spawn(read_lines(e, "stderr", tx.clone()));
    }
    drop(tx);
    let pump = tokio::spawn(pump_lines(store.clone(), svc.clone(), rx));

    let code = match app.wait().await {
        Ok(s) => s.code().unwrap_or(-1),
        Err(_) => -1,
    };
    let _ = pump.await;
    APP_PGID.store(0, Ordering::SeqCst);
    let msg = format!("exited ({code}): {shown}");
    store.push(Event::new("log", &svc).with("level", if code == 0 { "info" } else { "error" })
        .with("text", format!("process {msg}")).with("stream", "cdev"));
    store.set_app_status(msg);
    code
}

/// Stop the whole app process tree (npm → node → workers).
pub fn stop_app() {
    let pgid = APP_PGID.load(Ordering::SeqCst);
    if pgid <= 0 {
        return;
    }
    // SIGTERM to the process group
    #[cfg(unix)]
    let _ = std::process::Command::new("kill").args(["-TERM", &format!("-{pgid}")]).status();
    // Windows has no group signal a console-less child can receive; kill the tree
    #[cfg(windows)]
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pgid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

async fn read_lines<R: AsyncRead + Unpin>(r: R, stream: &'static str, tx: mpsc::UnboundedSender<(Instant, f64, &'static str, String)>) {
    let mut lines = BufReader::new(r).lines();
    while let Ok(Some(l)) = lines.next_line().await {
        if tx.send((Instant::now(), now_ms(), stream, l)).is_err() {
            break;
        }
    }
}

/// Hold each line briefly so an SDK-reported copy of the same console output
/// can arrive first and win (it carries types + source location).
async fn pump_lines(store: Arc<Store>, svc: String, mut rx: mpsc::UnboundedReceiver<(Instant, f64, &'static str, String)>) {
    while let Some((at, ts, stream, raw)) = rx.recv().await {
        tokio::time::sleep_until(at + Duration::from_millis(300)).await;
        let line = strip_ansi(&raw);
        if line.trim().is_empty() || store.consume_sdk_line(&line) {
            continue;
        }
        let mut e = line_event(&svc, stream, &line);
        e.ts = ts;
        store.push(e);
    }
}

fn line_event(svc: &str, stream: &str, line: &str) -> Event {
    let mut e = Event::new("log", svc).with("stream", stream);
    if line.trim_start().starts_with('{') {
        if let Ok(Value::Object(mut o)) = serde_json::from_str::<Value>(line.trim()) {
            let msg = o.remove("msg").or_else(|| o.remove("message"));
            if let Some(Value::String(m)) = msg {
                let level = match o.remove("level") {
                    Some(Value::Number(n)) => match n.as_f64().unwrap_or(30.0) as i64 {
                        n if n >= 50 => "error",
                        n if n >= 40 => "warn",
                        n if n >= 30 => "info",
                        _ => "debug",
                    }.to_string(),
                    Some(Value::String(s)) => s.to_lowercase(),
                    _ => "info".into(),
                };
                for k in ["time", "pid", "hostname", "v"] {
                    o.remove(k);
                }
                return e.with("level", level).with("text", m).with("fields", Value::Object(o));
            }
        }
    }
    let lower = line.to_lowercase();
    let level = if lower.contains("error") || lower.contains("exception") || lower.contains("traceback") || lower.contains("panic") {
        "error"
    } else if lower.contains("warn") {
        "warn"
    } else if stream == "stderr" {
        "stderr"
    } else {
        "info"
    };
    e = e.with("level", level).with("text", line.to_string());
    e
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            match it.peek() {
                Some('[') => {
                    it.next();
                    for c in it.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC: until BEL or ST
                    it.next();
                    while let Some(c) = it.next() {
                        if c == '\x07' || (c == '\x1b' && it.peek() == Some(&'\\')) {
                            break;
                        }
                    }
                }
                _ => {
                    it.next();
                }
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ansi() {
        assert_eq!(strip_ansi("\x1b[32mok\x1b[0m done"), "ok done");
    }
    #[test]
    fn pino() {
        let e = line_event("s", "stdout", r#"{"level":50,"msg":"boom","user":1}"#);
        assert_eq!(e.str("level"), "error");
        assert_eq!(e.str("text"), "boom");
    }
}
