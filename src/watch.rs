//! `cdev watch <file|dir>` — no dev server or project setup needed.
//!
//! * HTML file or folder: served on port+1 with the browser SDK and a reload
//!   hook injected; `.ts/.tsx/.jsx` are compiled on request (bun or esbuild);
//!   saving reloads the page (CSS-only changes swap stylesheets in place).
//! * Script (`.js .ts .py .cpp …`): runs instrumented, re-runs on every save.

use crate::event::Event;
use crate::runner;
use crate::store::Store;
use axum::{
    extract::{Path as UrlPath, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

const SKIP_DIRS: [&str; 8] = ["node_modules", ".git", "target", "dist", "build", "__pycache__", ".next", ".venv"];

pub async fn watch(store: Arc<Store>, target: PathBuf, port: u16) -> i32 {
    let target = match target.canonicalize() {
        Ok(t) => t,
        Err(e) => {
            let msg = format!("cannot watch {}: {e}", target.display());
            store.push(Event::new("error", "cdev").with("msg", msg.clone()));
            store.set_app_status(msg);
            return 1;
        }
    };
    let ext = target.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if target.is_dir() || ext == "html" || ext == "htm" {
        serve_page(store, target, port).await
    } else {
        rerun_script(store, target, &ext, port).await
    }
}

// ---------------------------------------------------------------- pages

#[derive(Clone)]
struct Site {
    root: PathBuf,
    sidecar: u16,
}

async fn serve_page(store: Arc<Store>, target: PathBuf, port: u16) -> i32 {
    if auto_watch() {
        let _ = runner::write_sdk(&runner::sdk_dir(), true); // bun build helper for .ts pages
    }
    let (root, page) = if target.is_dir() {
        (target.clone(), if target.join("index.html").exists() { "index.html".to_string() } else { String::new() })
    } else {
        (target.parent().unwrap_or(Path::new("/")).to_path_buf(), target.file_name().unwrap().to_string_lossy().into_owned())
    };
    let site_port = port + 1;
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", site_port)).await {
        Ok(l) => l,
        Err(e) => {
            let msg = format!("port {site_port} unavailable for the page server: {e}");
            store.push(Event::new("error", "cdev").with("msg", msg.clone()));
            store.set_app_status(msg);
            return 1;
        }
    };
    let app = Router::new()
        .route("/", get(|State(s): State<Site>| async move { serve(&s, "").await }))
        .route("/{*path}", get(|State(s): State<Site>, UrlPath(p): UrlPath<String>| async move { serve(&s, &p).await }))
        .with_state(Site { root: root.clone(), sidecar: port });
    tokio::spawn(async move { axum::serve(listener, app).await });

    let url = format!("http://localhost:{site_port}/{page}");
    store.set_app_status(format!("watching {} → {url}", root.display()));
    store.push(Event::new("log", "cdev").with("level", "info").with("stream", "cdev")
        .with("text", format!("serving {} at {url} (reloads on save)", root.display())));
    let _ = std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" })
        .arg(&url).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();

    let mut snap = scan(&root);
    loop {
        let changed = wait_change(&root, &mut snap).await;
        let css_only = changed.iter().all(|p| p.extension().and_then(|e| e.to_str()) == Some("css"));
        let names: Vec<String> = changed.iter().filter_map(|p| p.strip_prefix(&root).ok()).map(|p| p.display().to_string()).collect();
        store.push(Event::new("log", "cdev").with("level", "info").with("stream", "cdev")
            .with("text", format!("changed: {} → {}", names.join(", "), if css_only { "css swap" } else { "reload" })));
        let _ = store.reload.send(if css_only { "css".into() } else { "full".into() });
    }
}

async fn serve(site: &Site, path: &str) -> Response {
    let rel = Path::new(path);
    if rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return (StatusCode::FORBIDDEN, "nope").into_response();
    }
    let full = site.root.join(rel);
    if full.is_dir() {
        let index = full.join("index.html");
        if index.exists() {
            return serve_file(site, &index).await;
        }
        return listing(&site.root, &full);
    }
    serve_file(site, &full).await
}

async fn serve_file(site: &Site, full: &Path) -> Response {
    let ext = full.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let no_store = (header::CACHE_CONTROL, "no-store");
    match ext.as_str() {
        "html" | "htm" => match tokio::fs::read_to_string(full).await {
            Ok(html) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), no_store], inject(&html, site.sidecar)).into_response(),
            Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
        },
        "ts" | "tsx" | "jsx" | "mts" => {
            let js = match auto_watch() {
                true => compile_ts_auto(full, &site.root, site.sidecar).await,
                false => None,
            };
            let js = match js {
                Some(js) => js,
                None => compile_ts(full).await,
            };
            ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), no_store], js).into_response()
        }
        "js" | "mjs" if auto_watch() => match tokio::fs::read_to_string(full).await {
            Ok(src) => {
                let rel = full.strip_prefix(&site.root).unwrap_or(full).display().to_string();
                let out = crate::instrument::instrument(&src, &rel).unwrap_or(src);
                ([(header::CONTENT_TYPE, mime(&ext)), no_store], out).into_response()
            }
            Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
        },
        _ => match tokio::fs::read(full).await {
            Ok(bytes) => ([(header::CONTENT_TYPE, mime(&ext)), no_store], bytes).into_response(),
            Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
        },
    }
}

fn inject(html: &str, sidecar: u16) -> String {
    let snippet = format!(
        r#"<script src="http://127.0.0.1:{sidecar}/cdev.js"></script>
<script>addEventListener('load',()=>{{const es=new EventSource('http://127.0.0.1:{sidecar}/reload');es.onmessage=(m)=>{{if(m.data==='css'){{document.querySelectorAll('link[rel=stylesheet]').forEach((l)=>{{const u=new URL(l.href);u.searchParams.set('cdev',Date.now());l.href=u.href;}});}}else location.reload();}};}},{{once:true}});</script>
"#
    );
    // as early as possible so console/errors from page scripts are captured
    let lower = html.to_lowercase();
    if let Some(i) = lower.find("<head>") {
        let at = i + "<head>".len();
        return format!("{}\n{snippet}{}", &html[..at], &html[at..]);
    }
    if let Some(i) = lower.find("<html") {
        if let Some(j) = lower[i..].find('>') {
            let at = i + j + 1;
            return format!("{}\n{snippet}{}", &html[..at], &html[at..]);
        }
    }
    format!("{snippet}{html}")
}

fn auto_watch() -> bool {
    std::env::var_os("CDEV_AUTO_WATCH").is_some()
}

/// Bundle with bun, rewriting project files for auto-watch (None → fall back to plain compile).
async fn compile_ts_auto(file: &Path, root: &Path, sidecar: u16) -> Option<String> {
    let script = runner::sdk_dir().join("cdev-bun-build.js");
    let out = tokio::process::Command::new("bun")
        .arg(&script).arg(file).arg(root).arg(format!("http://127.0.0.1:{sidecar}"))
        .output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn compile_ts(file: &Path) -> String {
    let attempts: [(&str, Vec<String>); 2] = [
        ("bun", vec!["build".into(), file.display().to_string(), "--target=browser".into(), "--sourcemap=inline".into()]),
        ("esbuild", vec![file.display().to_string(), "--bundle".into(), "--format=esm".into(), "--sourcemap=inline".into()]),
    ];
    for (bin, args) in attempts {
        match tokio::process::Command::new(bin).args(&args).output().await {
            Ok(out) if out.status.success() => return String::from_utf8_lossy(&out.stdout).into_owned(),
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr).replace('`', "'");
                return format!("console.error(`cdev: {bin} failed to compile {}:\\n{}`);", file.display(), err.replace('\\', "\\\\"));
            }
            Err(_) => continue, // not installed, try next
        }
    }
    "console.error('cdev: install bun or esbuild to serve .ts/.tsx files');".into()
}

fn listing(root: &Path, dir: &Path) -> Response {
    let mut items = String::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut names: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| {
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap_or(&p).display().to_string();
            if p.is_dir() { format!("{rel}/") } else { rel }
        }).collect();
        names.sort();
        for n in names {
            items.push_str(&format!("<li><a href=\"/{n}\">{n}</a></li>"));
        }
    }
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        format!("<!doctype html><meta charset=utf-8><title>cdev</title><body style=\"font:14px ui-monospace,monospace;padding:24px\"><h3>{}</h3><ul>{items}</ul>", dir.display()),
    ).into_response()
}

fn mime(ext: &str) -> &'static str {
    match ext {
        "js" | "mjs" | "cjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "wasm" => "application/wasm",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------- scripts

fn has(bin: &str) -> bool {
    std::process::Command::new("sh").args(["-c", &format!("command -v {bin} >/dev/null")]).status().map_or(false, |s| s.success())
}

fn script_cmd(file: &Path, ext: &str) -> Result<Vec<String>, String> {
    let f = file.display().to_string();
    let sdk = runner::sdk_dir();
    Ok(match ext {
        "js" | "mjs" | "cjs" => vec!["node".into(), f],
        "ts" | "mts" | "cts" | "tsx" => {
            if has("bun") {
                vec!["bun".into(), "--preload".into(), sdk.join("cdev-node.js").display().to_string(), f]
            } else {
                vec!["npx".into(), "--yes".into(), "tsx".into(), f]
            }
        }
        "py" => vec![if has("python3") { "python3" } else { "python" }.into(), f],
        "cpp" | "cc" | "cxx" | "c++" => {
            let bin = std::env::temp_dir().join("cdev-watch-bin");
            let cxx = std::env::var("CXX").unwrap_or_else(|_| "c++".into());
            vec!["sh".into(), "-c".into(), format!(
                "{cxx} -std=c++17 -g -O0 -I'{}' '{f}' -o '{}' && exec '{}'",
                sdk.display(), bin.display(), bin.display()
            )]
        }
        // single-file Rust: build in a temp dir with cdev.rs next to it, so `mod cdev;` resolves
        "rs" => {
            let dir = std::env::temp_dir().join("cdev-watch-rs");
            let bin = dir.join("app");
            let name = file.file_name().unwrap().to_string_lossy().into_owned();
            let orig = file.parent().unwrap_or(Path::new("/")).display().to_string();
            vec!["sh".into(), "-c".into(), format!(
                "mkdir -p '{d}' && cp '{f}' '{d}/{name}' && cp '{}' '{d}/cdev.rs' && rustc --edition 2021 -g --remap-path-prefix '{d}'='{orig}' -o '{b}' '{d}/{name}' && exec '{b}'",
                sdk.join("cdev.rs").display(), d = dir.display(), b = bin.display()
            )]
        }
        "php" => vec!["php".into(), f],
        // single-file Go: temp module where `import "cdev"` resolves to the SDK
        "go" => {
            let dir = std::env::temp_dir().join("cdev-watch-go");
            let gomod = "module app\n\ngo 1.21\n\nrequire cdev v0.0.0\n\nreplace cdev => ./cdev\n";
            vec!["sh".into(), "-c".into(), format!(
                "mkdir -p '{d}/cdev' && cp '{f}' '{d}/main.go' && cp '{sdk}/cdev-go/cdev.go' '{sdk}/cdev-go/go.mod' '{d}/cdev/' && printf '{gomod}' > '{d}/go.mod' && cd '{d}' && go run .",
                d = dir.display(), sdk = sdk.display()
            )]
        }
        other => return Err(format!("don't know how to run .{other} files (supported: js ts py cpp rs go php, html)")),
    })
}

async fn rerun_script(store: Arc<Store>, file: PathBuf, ext: &str, port: u16) -> i32 {
    let cmd = match script_cmd(&file, ext) {
        Ok(c) => c,
        Err(msg) => {
            store.push(Event::new("error", "cdev").with("msg", msg.clone()));
            store.set_app_status(msg);
            return 1;
        }
    };
    let root = file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    if auto_watch() && !matches!(ext, "py" | "js" | "mjs" | "cjs" | "ts" | "mts" | "cts" | "tsx") {
        store.push(Event::new("log", "cdev").with("level", "warn").with("stream", "cdev")
            .with("text", format!("--auto supports Python, JS and TS; for .{ext} use CDEV_W / cdev_w! (debugger-based auto-watch is planned)")));
    }
    let mut snap = scan(&root);
    let mut run_no = 0;
    loop {
        run_no += 1;
        store.push(Event::new("log", "cdev").with("level", "info").with("stream", "cdev")
            .with("text", format!("── run #{run_no}: {name} ──")));
        let label = if cmd[0] == "sh" { format!("build + run {name}") } else { cmd.join(" ") };
        let mut run = tokio::spawn(runner::run_as(store.clone(), cmd.clone(), port, Some(label)));
        tokio::select! {
            _ = &mut run => {
                // finished on its own; wait for the next save
                wait_change(&root, &mut snap).await;
            }
            _ = wait_change(&root, &mut snap) => {
                runner::stop_app();
                let _ = run.await;
            }
        }
    }
}

// ---------------------------------------------------------------- file polling

fn scan(root: &Path) -> HashMap<PathBuf, SystemTime> {
    let mut out = HashMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) && out.len() < 5000 {
                    stack.push(p);
                }
            } else if let Ok(m) = md.modified() {
                out.insert(p, m);
            }
        }
    }
    out
}

/// Poll every 250ms; return the changed paths once edits settle.
async fn wait_change(root: &Path, snap: &mut HashMap<PathBuf, SystemTime>) -> Vec<PathBuf> {
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let now = scan(root);
        if now != *snap {
            tokio::time::sleep(Duration::from_millis(120)).await; // editors write in bursts
            let settled = scan(root);
            let mut changed: Vec<PathBuf> = settled.iter()
                .filter(|(p, m)| snap.get(*p) != Some(*m)).map(|(p, _)| p.clone()).collect();
            changed.extend(snap.keys().filter(|p| !settled.contains_key(*p)).cloned());
            *snap = settled;
            if !changed.is_empty() {
                return changed;
            }
        }
    }
}
