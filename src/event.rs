//! Event model shared by every SDK. Events are loose JSON objects with a few
//! well-known top-level fields; everything else lives in `data`.
//!
//! kinds:
//!   log     { level, text?, args?[], loc? }
//!   watch   { name, v, t?, loc? }
//!   call    { name, params?[], args?[], argTypes?[], ret?, retType?, err?, ms, caller?, loc? }
//!   http    { dir: in|out, method, url, status?, ms?, err?, from?, req{headers,body}, res{headers,body} }
//!   error   { msg, stack? }
//!   service { name, port }

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub ts: f64,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub svc: String,
    #[serde(flatten)]
    pub data: Map<String, Value>,
}

fn default_kind() -> String {
    "log".into()
}

impl Event {
    pub fn new(kind: &str, svc: &str) -> Self {
        Event { id: 0, ts: now_ms(), kind: kind.into(), svc: svc.into(), data: Map::new() }
    }

    pub fn with(mut self, k: &str, v: impl Into<Value>) -> Self {
        self.data.insert(k.into(), v.into());
        self
    }

    pub fn get(&self, k: &str) -> Option<&Value> {
        self.data.get(k)
    }

    pub fn str(&self, k: &str) -> &str {
        self.data.get(k).and_then(Value::as_str).unwrap_or("")
    }

    pub fn num(&self, k: &str) -> Option<f64> {
        self.data.get(k).and_then(Value::as_f64)
    }

    pub fn level(&self) -> &str {
        match self.kind.as_str() {
            "error" => "error",
            "log" => match self.str("level") {
                "" => "info",
                l => l,
            },
            "http" if self.num("status").map_or(false, |s| s >= 400.0) || self.get("err").is_some() => "error",
            "call" if self.get("err").is_some() => "error",
            _ => "info",
        }
    }

    /// One-line human summary used by the TUI, headless output and filtering.
    pub fn summary(&self) -> String {
        match self.kind.as_str() {
            "log" => {
                let t = self.str("text");
                if !t.is_empty() {
                    return t.to_string();
                }
                match self.get("args") {
                    Some(Value::Array(a)) => a.iter().map(|v| match v {
                        Value::String(s) => s.clone(),
                        v => compact(v, 200),
                    }).collect::<Vec<_>>().join(" "),
                    _ => self.str("msg").to_string(),
                }
            }
            "watch" => {
                let v = self.get("v").map(|v| compact(v, 160)).unwrap_or_default();
                format!("{} = {}  : {}", self.str("name"), v, self.str("shape"))
            }
            "call" => {
                let ret = match (self.get("err"), self.get("ret")) {
                    (Some(e), _) => format!("throws {}", e.as_str().unwrap_or("?")),
                    (None, Some(r)) => compact(r, 80),
                    _ => "void".into(),
                };
                format!("{}({}) → {}  {}", self.str("name"), self.args_inline(40), ret, fmt_ms(self.num("ms")))
            }
            "http" => {
                let arrow = if self.str("dir") == "in" { "⇠" } else { "⇢" };
                let status = match (self.num("status"), self.get("err")) {
                    (Some(s), _) => format!("{s}"),
                    (None, Some(e)) => format!("ERR {}", e.as_str().unwrap_or("")),
                    _ => "…".into(),
                };
                format!("{arrow} {} {} → {}  {}", self.str("method"), self.str("url"), status, fmt_ms(self.num("ms")))
            }
            "error" => format!("{}", self.str("msg")),
            "service" => format!("{} listening on :{}", self.str("name"), self.num("port").unwrap_or(0.0)),
            _ => compact(&Value::Object(self.data.clone()), 200),
        }
    }

    /// `id=42, opts={…}` using param names when the SDK sent them.
    pub fn args_inline(&self, per: usize) -> String {
        let args = match self.get("args") {
            Some(Value::Array(a)) => a,
            _ => return String::new(),
        };
        let params = self.get("params").and_then(Value::as_array);
        args.iter().enumerate().map(|(i, v)| {
            let n = params.and_then(|p| p.get(i)).and_then(Value::as_str).unwrap_or("");
            if n.is_empty() { compact(v, per) } else { format!("{n}={}", compact(v, per)) }
        }).collect::<Vec<_>>().join(", ")
    }

    /// `(id: number, opts: {…}) → User` for calls.
    pub fn signature(&self) -> String {
        let shapes = self.get("argShapes").and_then(Value::as_array);
        let params = self.get("params").and_then(Value::as_array);
        let n = shapes.map_or(0, |s| s.len());
        let parts: Vec<String> = (0..n).map(|i| {
            let s = shapes.and_then(|s| s.get(i)).and_then(Value::as_str).unwrap_or("?");
            match params.and_then(|p| p.get(i)).and_then(Value::as_str) {
                Some(p) if !p.is_empty() => format!("{p}: {s}"),
                _ => s.to_string(),
            }
        }).collect();
        format!("{}({}) → {}", self.str("name"), parts.join(", "), self.str("retShape"))
    }

    /// Compute type shapes server-side so every SDK gets them for free.
    pub fn enrich(&mut self) {
        if self.ts == 0.0 {
            self.ts = now_ms();
        }
        self.decode_debug();
        match self.kind.as_str() {
            "watch" => {
                let s = match self.str("t") {
                    "" => shape_of(self.get("v").unwrap_or(&Value::Null)),
                    t => t.to_string(),
                };
                self.data.insert("shape".into(), s.into());
                let v = self.get("v").unwrap_or(&Value::Null);
                if let Some(k) = crate::structure::detect(v, self.str("name")) {
                    self.data.insert("ds".into(), k.name().into());
                }
            }
            "call" | "log" => {
                if let Some(Value::Array(args)) = self.get("args") {
                    let types = self.get("argTypes").and_then(Value::as_array);
                    let shapes: Vec<Value> = args.iter().enumerate().map(|(i, v)| {
                        match types.and_then(|t| t.get(i)).and_then(Value::as_str) {
                            Some(t) if !t.is_empty() => t.into(),
                            _ => shape_of(v).into(),
                        }
                    }).collect();
                    self.data.insert("argShapes".into(), Value::Array(shapes));
                }
                if self.kind == "call" {
                    let rs = match (self.str("retType"), self.get("ret")) {
                        (t, _) if !t.is_empty() => t.to_string(),
                        (_, Some(r)) => shape_of(r),
                        _ => "void".into(),
                    };
                    self.data.insert("retShape".into(), rs.into());
                }
            }
            _ => {}
        }
    }
}

impl Event {
    /// The Rust SDK sends `{:?}` strings (`vdebug`, `argsDebug`, `retDebug`);
    /// turn them into JSON values so shapes and diagrams work unchanged.
    fn decode_debug(&mut self) {
        let dec = |s: &str| crate::debugfmt::parse(s).unwrap_or_else(|| Value::String(s.to_string()));
        if let Some(Value::String(d)) = self.data.remove("vdebug") {
            self.data.insert("v".into(), dec(&d));
        }
        if let Some(Value::String(d)) = self.data.remove("retDebug") {
            self.data.insert("ret".into(), dec(&d));
        }
        if let Some(Value::Array(a)) = self.data.remove("argsDebug") {
            let args: Vec<Value> = a.iter().map(|x| x.as_str().map(dec).unwrap_or(Value::Null)).collect();
            self.data.insert("args".into(), Value::Array(args));
        }
    }
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

pub fn fmt_ms(ms: Option<f64>) -> String {
    match ms {
        Some(m) if m >= 1000.0 => format!("{:.2}s", m / 1000.0),
        Some(m) => format!("{m:.1}ms"),
        None => String::new(),
    }
}

pub fn fmt_time(ts: f64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(ts as i64) {
        chrono::LocalResult::Single(t) => t.format("%H:%M:%S%.3f").to_string(),
        _ => "--:--:--".into(),
    }
}

/// Compact single-line JSON, truncated to `max` chars.
pub fn compact(v: &Value, max: usize) -> String {
    let s = match v {
        Value::Object(o) if o.contains_key("__t") => tagged_display(o),
        _ => serde_json::to_string(&without_meta(v)).unwrap_or_default(),
    };
    truncate(&s, max)
}

/// Drop SDK bookkeeping keys (`__class`, `__id`) for display.
fn without_meta(v: &Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(o.iter()
            .filter(|(k, _)| k.as_str() != "__class" && k.as_str() != "__id")
            .map(|(k, x)| (k.clone(), without_meta(x))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(without_meta).collect()),
        v => v.clone(),
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn tagged_display(o: &Map<String, Value>) -> String {
    let t = o.get("__t").and_then(Value::as_str).unwrap_or("?");
    match t {
        "undefined" => "undefined".into(),
        "function" => format!("ƒ {}", o.get("name").and_then(Value::as_str).unwrap_or("")),
        "circular" => "[Circular]".into(),
        _ => match o.get("v").or_else(|| o.get("message")) {
            Some(Value::String(s)) => format!("{t}({s})"),
            _ => serde_json::to_string(o).unwrap_or_default(),
        },
    }
}

/// Infer a TypeScript-ish type shape from a JSON value produced by an SDK.
pub fn shape_of(v: &Value) -> String {
    shape(v, 0, &mut Vec::new())
}

/// `classes` holds the class names of enclosing objects, so self-referential
/// types print as `ListNode { next: ListNode }` instead of expanding forever.
fn shape(v: &Value, depth: usize, classes: &mut Vec<String>) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(_) => "boolean".into(),
        Value::Number(_) => "number".into(),
        Value::String(_) => "string".into(),
        Value::Array(a) => {
            let items: Vec<&Value> = a.iter().filter(|x| !is_tag(x, "more")).collect();
            if items.is_empty() {
                return "[]".into();
            }
            let mut uniq: Vec<String> = Vec::new();
            for x in items.iter().take(20) {
                let s = shape(x, depth + 1, classes);
                if !uniq.contains(&s) {
                    uniq.push(s);
                }
            }
            if uniq.len() == 1 {
                let s = &uniq[0];
                if s.contains(" | ") { format!("({s})[]") } else { format!("{s}[]") }
            } else {
                format!("({})[]", uniq.join(" | "))
            }
        }
        Value::Object(o) => {
            if o.contains_key("__ref") {
                return "↺ (ref)".into();
            }
            if let Some(t) = o.get("__t").and_then(Value::as_str) {
                return match t {
                    "function" => "function".into(),
                    "Map" => {
                        let (k, val) = o.get("entries").and_then(Value::as_array)
                            .and_then(|e| e.first()).and_then(Value::as_array)
                            .map(|p| (shape(&p[0], depth + 1, classes), shape(p.get(1).unwrap_or(&Value::Null), depth + 1, classes)))
                            .unwrap_or(("unknown".into(), "unknown".into()));
                        format!("Map<{k}, {val}>")
                    }
                    "Set" => {
                        let inner = o.get("values").and_then(Value::as_array)
                            .and_then(|a| a.first()).map(|x| shape(x, depth + 1, classes)).unwrap_or("unknown".into());
                        format!("Set<{inner}>")
                    }
                    other => other.to_string(),
                };
            }
            let class = o.get("__class").and_then(Value::as_str);
            if let Some(c) = class {
                if classes.iter().any(|x| x == c) {
                    return c.to_string();
                }
            }
            if depth >= 3 {
                return class.map(str::to_string).unwrap_or("{…}".into());
            }
            let keys: Vec<(&String, &Value)> = o.iter().filter(|(k, _)| !matches!(k.as_str(), "__class" | "__id")).collect();
            if let Some(c) = class {
                classes.push(c.to_string());
            }
            // a nullable link to the same class reads as `T | null`
            let mut parts: Vec<String> = keys.iter().take(8).map(|(k, v)| {
                let s = match (class, v) {
                    (Some(c), Value::Null) if ["next", "prev", "left", "right", "parent", "child"].contains(&k.as_str()) => format!("{c} | null"),
                    _ => shape(v, depth + 1, classes),
                };
                format!("{k}: {s}")
            }).collect();
            if class.is_some() {
                classes.pop();
            }
            if keys.len() > 8 {
                parts.push(format!("…+{}", keys.len() - 8));
            }
            let body = if parts.is_empty() { "{}".to_string() } else { format!("{{ {} }}", parts.join(", ")) };
            match class {
                Some(c) => format!("{c} {body}"),
                None => body,
            }
        }
    }
}

fn is_tag(v: &Value, t: &str) -> bool {
    v.get("__t").and_then(Value::as_str) == Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shapes() {
        assert_eq!(shape_of(&json!({"id": 1, "tags": ["a"]})), "{ id: number, tags: string[] }");
        assert_eq!(shape_of(&json!([1, "a"])), "(number | string)[]");
        assert_eq!(shape_of(&json!({"__class": "User", "n": "x"})), "User { n: string }");
        assert_eq!(shape_of(&json!({"__t": "Map", "entries": [["k", 2]]})), "Map<string, number>");
        let list = json!({"__class": "ListNode", "val": 1, "next": {"__class": "ListNode", "val": 2, "next": null}});
        assert_eq!(shape_of(&list), "ListNode { val: number, next: ListNode }");
        let one = json!({"__class": "ListNode", "val": 1, "next": null});
        assert_eq!(shape_of(&one), "ListNode { val: number, next: ListNode | null }");
    }
}
