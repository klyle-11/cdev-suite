mod api;
mod debugfmt;
mod event;
mod explain;
mod instrument;
mod memory;
mod runner;
mod server;
mod store;
mod structure;
mod tui;
mod watch;

use anyhow::{bail, Context, Result};
use std::sync::Arc;
use store::Store;

const USAGE: &str = "\
cdev — dev-time observability sidecar

USAGE:
  cdev [--port N] [--headless] [--cap N]              start sidecar + TUI (web panel on :port)
  cdev [--port N] [--headless] run -- <cmd> [args…]   wrap a command, auto-instrument Node/Python
  cdev [--port N] [--headless] watch <file|dir>        serve+reload an HTML page, or re-run a .js/.ts/.py/.cpp file on save
  cdev --auto [--only f,g] watch|run …                  auto-watch every local in your code (Python for now)
  cdev explain <file>                                 read a JS/TS file without running it (opens the Code tab)
  cdev explain --text|--json [--name label] [file]    …print the static report instead (stdin if no file)
  cdev instrument [--name label] [file]               print the JS/TS auto-watch rewrite (stdin if no file)
  cdev sdk <dir>                                      copy SDK files (node, browser, python, c++, d.ts)

OPTIONS:
  --port N     listen port (default 4400, env CDEV_PORT)
  --headless   no TUI; print one line per event to stdout
  --cap N      events kept in memory (default 20000)
  --auto       record every local variable after each statement, no cdev.w() needed (Python, JS, TS)
  --only f,g   with --auto: only these functions
";

struct Args {
    port: u16,
    headless: bool,
    cap: usize,
    cmd: Option<Vec<String>>,
    watch: Option<std::path::PathBuf>,
    explain: Option<std::path::PathBuf>,
    auto: bool,
}

fn parse() -> Result<Args> {
    let mut a = Args {
        port: std::env::var("CDEV_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(4400),
        headless: false,
        cap: 20_000,
        cmd: None,
        watch: None,
        explain: None,
        auto: false,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-h" | "--help" | "help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("cdev {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--port" | "-p" => {
                i += 1;
                a.port = argv.get(i).context("--port needs a value")?.parse().context("bad port")?;
            }
            "--cap" => {
                i += 1;
                a.cap = argv.get(i).context("--cap needs a value")?.parse().context("bad cap")?;
            }
            "--headless" => a.headless = true,
            "--auto" => a.auto = true,
            "--only" => {
                i += 1;
                let fns = argv.get(i).context("--only needs a list of function names")?;
                std::env::set_var("CDEV_ONLY", fns);
            }
            "instrument" => {
                // print the auto-watch rewrite of a JS/TS file (stdin with --name)
                let rest = &argv[i + 1..];
                let name = rest.iter().position(|a| a == "--name").and_then(|p| rest.get(p + 1)).cloned();
                let path = rest.iter().find(|a| !a.starts_with("--") && Some(*a) != name.as_ref()).cloned();
                let src = match &path {
                    Some(p) => std::fs::read_to_string(p)?,
                    None => std::io::read_to_string(std::io::stdin())?,
                };
                let label = name.or(path).unwrap_or_else(|| "stdin.js".into());
                match instrument::instrument(&src, &label) {
                    Ok(out) => print!("{out}"),
                    Err(e) => {
                        eprintln!("cdev instrument: {label}: {e} (left unchanged)");
                        print!("{src}");
                    }
                }
                std::process::exit(0);
            }
            "explain" => {
                // static report of a JS/TS file: the Code tab on a terminal, text or JSON otherwise
                use std::io::IsTerminal;
                let rest = &argv[i + 1..];
                let json = rest.iter().any(|a| a == "--json");
                let print = json || rest.iter().any(|a| a == "--text") || !std::io::stdout().is_terminal();
                let name = rest.iter().position(|a| a == "--name").and_then(|p| rest.get(p + 1)).cloned();
                let path = rest.iter().find(|a| !a.starts_with("--") && Some(*a) != name.as_ref()).cloned();
                let label = name.or(path.clone()).context("usage: cdev explain <file>   (or --name <file> with the source on stdin)")?;
                if !explain::supported(&label) {
                    bail!("cdev explain: {label}: the static view reads JS/TS for now");
                }
                if let (Some(p), false) = (&path, print) {
                    if !std::path::Path::new(p).is_file() {
                        bail!("cdev explain: {p}: no such file");
                    }
                    a.explain = Some(p.into());
                    break;
                }
                let src = match &path {
                    Some(p) => std::fs::read_to_string(p).with_context(|| format!("cdev explain: {p}"))?,
                    None => std::io::read_to_string(std::io::stdin())?,
                };
                match explain::explain(&src, &label) {
                    Ok(r) if json => println!("{}", serde_json::to_string(&r)?),
                    Ok(r) => print!("{}", explain::text(&r)),
                    Err(e) => {
                        eprintln!("cdev explain: {label}: {e}");
                        std::process::exit(1);
                    }
                }
                std::process::exit(0);
            }
            "sdk" => {
                let dir = argv.get(i + 1).map(String::as_str).unwrap_or(".");
                runner::write_sdk(std::path::Path::new(dir), false)?;
                println!("wrote cdev-node.js, cdev.js, cdev.d.ts, cdev.py, cdev.hpp, cdev.rs, cdev.php, cdev-go/ to {dir}");
                std::process::exit(0);
            }
            "watch" => {
                let p = argv.get(i + 1).context("usage: cdev watch <file|dir>")?;
                a.watch = Some(p.into());
                break;
            }
            "run" => {
                let rest: Vec<String> = argv[i + 1..].iter().skip_while(|s| *s == "--").cloned().collect();
                if rest.is_empty() {
                    bail!("usage: cdev run -- <cmd> [args…]");
                }
                a.cmd = Some(rest);
                break;
            }
            other => bail!("unknown argument `{other}`\n\n{USAGE}"),
        }
        i += 1;
    }
    Ok(a)
}

fn main() -> Result<()> {
    let args = parse()?;
    if args.auto {
        // read by the Python SDK's sitecustomize hook in the wrapped app
        std::env::set_var("CDEV_AUTO_WATCH", "1");
    }
    let rt = tokio::runtime::Runtime::new()?;
    let store = Arc::new(Store::new(args.cap));

    // static view: nothing runs and nothing listens
    if let Some(path) = args.explain {
        let _guard = rt.enter();
        return tui::run(store, tui::Info { port: 0, cmd: None, code: Some(path) });
    }

    let listener = rt
        .block_on(tokio::net::TcpListener::bind(("127.0.0.1", args.port)))
        .with_context(|| format!("port {} is in use (is another cdev running? try --port)", args.port))?;
    let app = server::router(store.clone());
    rt.spawn(async move { axum::serve(listener, app).await });

    let app = match (&args.cmd, &args.watch) {
        (Some(cmd), _) => Some(rt.spawn(runner::run(store.clone(), cmd.clone(), args.port))),
        (_, Some(path)) => Some(rt.spawn(watch::watch(store.clone(), path.clone(), args.port))),
        _ => None,
    };
    let code = args.watch.clone().filter(|p| p.is_file() && explain::supported(&p.to_string_lossy()));
    let label = args.cmd.map(|c| c.join(" ")).or(args.watch.map(|p| format!("watch {}", p.display())));

    let code = if args.headless {
        rt.block_on(headless(store, args.port, app))
    } else {
        let _guard = rt.enter();
        let res = tui::run(store, tui::Info { port: args.port, cmd: label, code });
        runner::stop_app();
        res?;
        0
    };
    runner::stop_app();
    rt.shutdown_timeout(std::time::Duration::from_millis(300));
    std::process::exit(code);
}

async fn headless(store: Arc<Store>, port: u16, app: Option<tokio::task::JoinHandle<i32>>) -> i32 {
    eprintln!("cdev listening on http://127.0.0.1:{port}  (web panel, /api/events, /ingest)");
    let mut rx = store.tx.subscribe();
    let printer = async {
        loop {
            match rx.recv().await {
                Ok(e) if e.kind != "_clear" => {
                    let loc = e.str("loc");
                    let loc = if loc.is_empty() { String::new() } else { format!("  @{loc}") };
                    println!("{} {:<12} {:<6} {}{}", event::fmt_time(e.ts), event::truncate(&e.svc, 12), e.kind, e.summary(), loc);
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => eprintln!("… dropped {n} events"),
                Err(_) => break,
            }
        }
    };
    tokio::select! {
        _ = printer => 0,
        _ = tokio::signal::ctrl_c() => 130,
        // the printer keeps running during the grace sleep, so trailing SDK batches still print
        code = async {
            let code = match app { Some(h) => h.await.unwrap_or(1), None => std::future::pending().await };
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            code
        } => code,
    }
}
