//! Bounded in-memory event log + broadcast fan-out, plus the derived views
//! (vars, api calls, architecture edges) that both frontends render.

use crate::event::{now_ms, Event};
use serde::Serialize;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

pub struct Store {
    inner: Mutex<Inner>,
    pub tx: broadcast::Sender<Arc<Event>>,
    /// `cdev watch` page reloads: "css" or "full"
    pub reload: broadcast::Sender<String>,
}

pub struct Inner {
    pub events: VecDeque<Arc<Event>>,
    pub cap: usize,
    pub total: u64,
    next_id: u64,
    /// localhost port → service name, learned from `service` events.
    pub ports: HashMap<u16, String>,
    /// Lines recently logged through an SDK console hook, used to drop the
    /// duplicate copy the process wrapper sees on stdout.
    sdk_lines: VecDeque<(f64, String)>,
    pub app_status: Option<String>,
}

impl Store {
    pub fn new(cap: usize) -> Self {
        let (tx, _) = broadcast::channel(4096);
        let (reload, _) = broadcast::channel(16);
        Store {
            inner: Mutex::new(Inner {
                events: VecDeque::with_capacity(cap.min(4096)),
                cap,
                total: 0,
                next_id: 1,
                ports: HashMap::new(),
                sdk_lines: VecDeque::new(),
                app_status: None,
            }),
            tx,
            reload,
        }
    }

    pub fn push(&self, mut e: Event) -> Arc<Event> {
        e.enrich();
        // show paths relative to where cdev was started
        if let Some(serde_json::Value::String(l)) = e.data.get_mut("loc") {
            if let Some(rest) = l.strip_prefix(cwd_prefix()) {
                *l = rest.to_string();
            }
        }
        let mut g = self.inner.lock().unwrap();
        e.id = g.next_id;
        g.next_id += 1;
        g.total += 1;
        if e.kind == "service" {
            if let Some(p) = e.num("port") {
                let name = match e.str("name") { "" => e.svc.clone(), n => n.to_string() };
                g.ports.insert(p as u16, name);
            }
        }
        if e.kind == "log" && e.str("stream").is_empty() {
            for line in e.str("text").lines() {
                g.sdk_lines.push_back((e.ts, line.trim_end().to_string()));
            }
            while g.sdk_lines.len() > 500 {
                g.sdk_lines.pop_front();
            }
        }
        let e = Arc::new(e);
        if g.events.len() >= g.cap {
            g.events.pop_front();
        }
        g.events.push_back(e.clone());
        drop(g);
        let _ = self.tx.send(e.clone());
        e
    }

    /// True if an SDK already reported this line recently. Matches suffixes
    /// too, so `INFO:app:hello` or `[12:00] hello` pair with the SDK's `hello`.
    pub fn consume_sdk_line(&self, line: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        let cutoff = now_ms() - 5000.0;
        while g.sdk_lines.front().map_or(false, |(t, _)| *t < cutoff) {
            g.sdk_lines.pop_front();
        }
        let line = line.trim_end();
        if let Some(i) = g.sdk_lines.iter().position(|(_, l)| l == line || (l.len() >= 6 && line.ends_with(l.as_str()))) {
            g.sdk_lines.remove(i);
            true
        } else {
            false
        }
    }

    pub fn with<R>(&self, f: impl FnOnce(&Inner) -> R) -> R {
        f(&self.inner.lock().unwrap())
    }

    pub fn set_app_status(&self, s: String) {
        self.inner.lock().unwrap().app_status = Some(s);
    }

    pub fn clear(&self) {
        let mut g = self.inner.lock().unwrap();
        g.events.clear();
        drop(g);
        let _ = self.tx.send(Arc::new(Event::new("_clear", "cdev")));
    }
}

// ---------- derived views ----------

pub struct VarInfo {
    pub svc: String,
    pub name: String,
    pub count: usize,
    pub last: Arc<Event>,
}

/// Watched variables in order of first appearance.
pub fn vars(inner: &Inner) -> Vec<VarInfo> {
    let mut idx: HashMap<(&str, &str), usize> = HashMap::new();
    let mut out: Vec<VarInfo> = Vec::new();
    for e in inner.events.iter().filter(|e| e.kind == "watch") {
        let key = (e.svc.as_str(), e.str("name"));
        match idx.get(&key) {
            Some(&i) => {
                out[i].count += 1;
                out[i].last = e.clone();
            }
            None => {
                idx.insert(key, out.len());
                out.push(VarInfo { svc: e.svc.clone(), name: e.str("name").to_string(), count: 1, last: e.clone() });
            }
        }
    }
    out
}

pub fn var_history(inner: &Inner, svc: &str, name: &str) -> Vec<Arc<Event>> {
    inner.events.iter()
        .filter(|e| e.kind == "watch" && e.svc == svc && e.str("name") == name)
        .cloned().collect()
}

#[derive(Serialize, Default, Clone)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// "http" or "call"
    pub via: String,
    pub count: u64,
    pub errors: u64,
    pub avg_ms: f64,
    /// top routes / call names on this edge with counts
    pub labels: Vec<(String, u64)>,
    #[serde(skip)]
    out_n: u64,
    #[serde(skip)]
    in_n: u64,
    #[serde(skip)]
    ms_sum: f64,
    #[serde(skip)]
    ms_n: u64,
    #[serde(skip)]
    /// label → (out count, in count)
    label_map: HashMap<String, (u64, u64)>,
}

/// Who talks to whom. HTTP edges are seen from both ends (client `out`,
/// server `in`), so each edge counts max(out, in) instead of the sum.
pub fn edges(inner: &Inner) -> Vec<Edge> {
    let mut map: HashMap<(String, String, &'static str), Edge> = HashMap::new();
    for e in inner.events.iter() {
        let (from, to, via, label) = match e.kind.as_str() {
            "http" => {
                let path = route_of(&path_of(e.str("url")));
                let label = format!("{} {}", e.str("method"), path);
                if e.str("dir") == "in" {
                    let from = match e.str("from") { "" => "client".to_string(), f => f.to_string() };
                    (from, e.svc.clone(), "http", label)
                } else {
                    (e.svc.clone(), resolve_host(inner, e.str("url")), "http", label)
                }
            }
            "call" => {
                // caller is the enclosing traced fn, or "GET /route" for request handlers
                let from = match e.str("caller") {
                    "" => e.svc.clone(),
                    p => format!("{}·{}", e.svc, caller_route(p)),
                };
                (from, format!("{}·{}", e.svc, e.str("name")), "call", e.str("name").to_string())
            }
            _ => continue,
        };
        let ed = map.entry((from.clone(), to.clone(), via)).or_insert_with(|| Edge {
            from, to, via: via.into(), ..Default::default()
        });
        if e.str("dir") == "in" { ed.in_n += 1 } else { ed.out_n += 1 }
        if e.level() == "error" {
            ed.errors += 1;
        }
        if let Some(ms) = e.num("ms") {
            ed.ms_sum += ms;
            ed.ms_n += 1;
        }
        let l = ed.label_map.entry(label).or_default();
        if e.str("dir") == "in" { l.1 += 1 } else { l.0 += 1 }
    }
    let mut out: Vec<Edge> = map.into_values().map(|mut e| {
        e.count = e.out_n.max(e.in_n);
        e.avg_ms = if e.ms_n > 0 { e.ms_sum / e.ms_n as f64 } else { 0.0 };
        let mut l: Vec<(String, u64)> = e.label_map.drain().map(|(k, (o, i))| (k, o.max(i))).collect();
        l.sort_by(|a, b| b.1.cmp(&a.1));
        l.truncate(6);
        e.labels = l;
        e
    }).collect();
    out.sort_by(|a, b| a.from.cmp(&b.from).then(b.count.cmp(&a.count)));
    out
}

fn split_url(url: &str) -> (&str, &str) {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    match rest.find('/') {
        Some(i) if url.contains("://") => (&rest[..i], &rest[i..]),
        _ if url.contains("://") => (rest, "/"),
        _ => ("", url),
    }
}

pub fn path_of(url: &str) -> String {
    let (_, p) = split_url(url);
    p.split(['?', '#']).next().unwrap_or("/").to_string()
}

/// `/api/users/42/posts/9f8c…` → `/api/users/:id/posts/:id` so routes group.
pub fn route_of(path: &str) -> String {
    path.split('/').map(|seg| {
        let digits = !seg.is_empty() && seg.chars().all(|c| c.is_ascii_digit());
        let hexish = seg.len() >= 16 && seg.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
        if digits || hexish { ":id" } else { seg }
    }).collect::<Vec<_>>().join("/")
}

/// Normalise the route part of a caller like `GET /api/users/2`.
fn caller_route(c: &str) -> String {
    match c.split_once(' ') {
        Some((m, p)) if p.starts_with('/') => format!("{m} {}", route_of(p)),
        _ => c.to_string(),
    }
}

pub fn resolve_host(inner: &Inner, url: &str) -> String {
    let (host, _) = split_url(url);
    if host.is_empty() {
        return "?".into();
    }
    let (h, port) = match host.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => (h, p.parse::<u16>().ok()),
        _ => (host, None),
    };
    let local = matches!(h, "localhost" | "127.0.0.1" | "[::1]" | "0.0.0.0");
    if local {
        if let Some(name) = port.and_then(|p| inner.ports.get(&p)) {
            return name.clone();
        }
    }
    host.to_string()
}

/// Filter used by `/api/events` and the TUI.
pub fn matches(e: &Event, kind: Option<&str>, q: &str) -> bool {
    if let Some(k) = kind {
        if !k.is_empty() && !k.split(',').any(|k| k == e.kind || k == e.level()) {
            return false;
        }
    }
    if q.is_empty() {
        return true;
    }
    let q = q.to_lowercase();
    e.svc.to_lowercase().contains(&q)
        || e.summary().to_lowercase().contains(&q)
        || e.str("loc").to_lowercase().contains(&q)
}

fn cwd_prefix() -> &'static str {
    static P: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    P.get_or_init(|| std::env::current_dir().map(|d| format!("{}/", d.display())).unwrap_or_default())
}
