//! Recognise classic data structures inside watched values and render them
//! as diagrams, so stepping through a variable's history animates the
//! algorithm operating on it.
//!
//! Detection works on plain JSON, so it is language-agnostic: a JS
//! `{val, next}` chain, a Python `TreeNode(left, right)` and a C++
//! `std::vector<std::vector<int>>` all get recognised.
//!
//! Shared/cyclic nodes arrive from the SDKs as `{"__ref": n}` pointing at an
//! object tagged `"__id": n`.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

const NEXT_KEYS: [&str; 3] = ["next", "nxt", "link"];
const VAL_KEYS: [&str; 6] = ["val", "value", "data", "key", "v", "item"];
const META_KEYS: [&str; 3] = ["__id", "__class", "__ref"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Array,
    Pointers,
    Matrix,
    List,
    Tree,
    NTree,
    Graph,
    Table,
    Heap,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Array => "array",
            Kind::Pointers => "array+pointers",
            Kind::Matrix => "matrix",
            Kind::List => "linked list",
            Kind::Tree => "binary tree",
            Kind::NTree => "tree",
            Kind::Graph => "graph",
            Kind::Table => "hash map",
            Kind::Heap => "heap",
        }
    }
}

pub fn detect(v: &Value, name: &str) -> Option<Kind> {
    let lname = name.to_lowercase();
    match v {
        Value::Array(a) if !a.is_empty() => {
            if a.iter().all(is_prim) {
                return Some(if lname.contains("heap") || lname.contains("pq") { Kind::Heap } else { Kind::Array });
            }
            if let Some(rows) = rows_of(a) {
                let w = rows[0].len();
                if w > 0 && rows.iter().all(|r| r.len() == w) {
                    return Some(Kind::Matrix);
                }
                let n = a.len() as f64;
                if rows.iter().all(|r| r.iter().all(|x| x.as_f64().map_or(false, |f| f >= 0.0 && f < n && f.fract() == 0.0))) {
                    return Some(Kind::Graph);
                }
            }
            None
        }
        Value::Object(o) => {
            match o.get("__t").and_then(Value::as_str) {
                Some("Map") | Some("Set") => return Some(Kind::Table),
                Some(_) => return None,
                None => {}
            }
            if next_key(o).is_some() {
                return Some(Kind::List);
            }
            if o.contains_key("left") || o.contains_key("right") {
                return Some(Kind::Tree);
            }
            if matches!(o.get("children"), Some(Value::Array(_))) {
                return Some(Kind::NTree);
            }
            if pointer_parts(o).is_some() {
                return Some(Kind::Pointers);
            }
            if is_adjacency(o) {
                return Some(Kind::Graph);
            }
            // wrapper objects like `{ head: … }` / `{ root: … }`
            for k in ["head", "root", "first", "top"] {
                if let Some(inner) = o.get(k) {
                    if let Some(kind) = detect(inner, "") {
                        if matches!(kind, Kind::List | Kind::Tree | Kind::NTree) {
                            return Some(kind);
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn is_prim(v: &Value) -> bool {
    matches!(v, Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_))
}

fn rows_of(a: &[Value]) -> Option<Vec<&Vec<Value>>> {
    a.iter().map(|r| match r {
        Value::Array(x) if x.iter().all(is_prim) => Some(x),
        _ => None,
    }).collect()
}

fn next_key(o: &Map<String, Value>) -> Option<&'static str> {
    NEXT_KEYS.into_iter().find(|k| matches!(o.get(*k), Some(Value::Object(_) | Value::Null)))
}

fn val_of(o: &Map<String, Value>) -> String {
    for k in VAL_KEYS {
        if let Some(v) = o.get(k) {
            return prim_text(v);
        }
    }
    o.iter().find(|(k, v)| !META_KEYS.contains(&k.as_str()) && is_prim(v))
        .map(|(_, v)| prim_text(v)).unwrap_or_else(|| "·".into())
}

fn val_num(o: &Map<String, Value>) -> Option<f64> {
    VAL_KEYS.iter().find_map(|k| o.get(*k)).and_then(Value::as_f64)
}

pub fn prim_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        v if is_prim(v) => v.to_string(),
        Value::Object(o) if o.contains_key("__ref") => "↺".into(),
        _ => "…".into(),
    }
}

/// `{arr: [...], i: 2, j: 5, swapped: true}` → (array key, pointers, extras)
fn pointer_parts(o: &Map<String, Value>) -> Option<(&str, Vec<(String, i64)>, Vec<(String, String)>)> {
    let arrays: Vec<(&String, &Vec<Value>)> = o.iter().filter_map(|(k, v)| match v {
        Value::Array(a) if !a.is_empty() && a.iter().all(is_prim) => Some((k, a)),
        _ => None,
    }).collect();
    if arrays.len() != 1 {
        return None;
    }
    let (ak, arr) = arrays[0];
    let mut ptrs = Vec::new();
    let mut extras = Vec::new();
    for (k, v) in o {
        if k == ak || META_KEYS.contains(&k.as_str()) {
            continue;
        }
        match v.as_i64() {
            Some(n) if n >= -1 && n <= arr.len() as i64 => ptrs.push((k.clone(), n)),
            _ => extras.push((k.clone(), prim_text(v))),
        }
    }
    if ptrs.is_empty() {
        return None;
    }
    Some((ak.as_str(), ptrs, extras))
}

fn is_adjacency(o: &Map<String, Value>) -> bool {
    let entries: Vec<(&String, &Value)> = o.iter().filter(|(k, _)| !META_KEYS.contains(&k.as_str())).collect();
    if entries.len() < 2 {
        return false;
    }
    let mut refs = 0usize;
    let mut hits = 0usize;
    for (_, v) in &entries {
        let Value::Array(a) = v else { return false };
        for x in a {
            let key = match x {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => return false,
            };
            refs += 1;
            if o.contains_key(&key) {
                hits += 1;
            }
        }
    }
    refs > 0 && hits * 2 >= refs
}

/// Collect `__id` → node for resolving `__ref`s.
fn index_ids<'a>(v: &'a Value, out: &mut HashMap<i64, &'a Value>) {
    match v {
        Value::Object(o) => {
            if let Some(id) = o.get("__id").and_then(Value::as_i64) {
                out.insert(id, v);
            }
            o.values().for_each(|x| index_ids(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| index_ids(x, out)),
        _ => {}
    }
}

fn unwrap_root<'a>(v: &'a Value, want: Kind) -> &'a Value {
    if let Value::Object(o) = v {
        for k in ["head", "root", "first", "top"] {
            if let Some(inner) = o.get(k) {
                if detect(inner, "") == Some(want) {
                    return inner;
                }
            }
        }
    }
    v
}

// ---------- rendering ----------

pub struct View {
    pub kind: Kind,
    pub note: String,
    pub lines: Vec<Line<'static>>,
}

/// Render `v` as a diagram, highlighting what changed since `prev`.
pub fn render(v: &Value, prev: Option<&Value>, name: &str, width: usize) -> Option<View> {
    let kind = detect(v, name)?;
    let prev = prev.filter(|p| detect(p, name) == Some(kind));
    let mut note = String::new();
    let lines = match kind {
        Kind::Array | Kind::Heap => {
            let a = v.as_array()?;
            let changed = diff_idx(a, prev.and_then(Value::as_array), &mut note);
            let mut l = cells(a, &changed, &[], width);
            if kind == Kind::Heap {
                l.push(Line::from(""));
                l.push(Line::from(Span::styled("as a tree (children of i at 2i+1, 2i+2):", Style::new().dim())));
                l.extend(heap_tree(a, &changed));
            }
            l
        }
        Kind::Pointers => {
            let o = v.as_object()?;
            let (ak, ptrs, extras) = pointer_parts(o)?;
            let a = o.get(ak)?.as_array()?;
            let pa = prev.and_then(|p| p.get(ak)).and_then(Value::as_array);
            let changed = diff_idx(a, pa, &mut note);
            let mut l = vec![Line::from(Span::styled(ak.to_string(), Style::new().bold()))];
            l.extend(cells(a, &changed, &ptrs, width));
            if !extras.is_empty() {
                let ex = extras.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("  ");
                l.push(Line::from(Span::styled(ex, Style::new().dim())));
            }
            l
        }
        Kind::Matrix => matrix(v.as_array()?, prev.and_then(Value::as_array), &mut note),
        Kind::List => list(unwrap_root(v, Kind::List), &mut note),
        Kind::Tree => tree(unwrap_root(v, Kind::Tree), &mut note),
        Kind::NTree => ntree(unwrap_root(v, Kind::NTree), &mut note),
        Kind::Graph => graph(v, &mut note),
        Kind::Table => table(v, &mut note),
    };
    Some(View { kind, note, lines })
}

fn diff_idx(a: &[Value], prev: Option<&Vec<Value>>, note: &mut String) -> HashSet<usize> {
    let Some(p) = prev else { return HashSet::new() };
    let changed: HashSet<usize> = (0..a.len()).filter(|&i| p.get(i) != a.get(i)).collect();
    let mut c: Vec<usize> = changed.iter().copied().collect();
    c.sort();
    if a.len() != p.len() {
        *note = format!("length {} → {}", p.len(), a.len());
    } else if c.len() == 2 && p[c[0]] == a[c[1]] && p[c[1]] == a[c[0]] {
        *note = format!("swap [{}] ↔ [{}]", c[0], c[1]);
    } else if c.len() == 1 {
        *note = format!("[{}]: {} → {}", c[0], prim_text(&p[c[0]]), prim_text(&a[c[0]]));
    } else if !c.is_empty() {
        *note = format!("{} cells changed", c.len());
    }
    changed
}

/// Boxed cells with indices above and named pointers below, wrapped to width.
fn cells(a: &[Value], changed: &HashSet<usize>, ptrs: &[(String, i64)], width: usize) -> Vec<Line<'static>> {
    let texts: Vec<String> = a.iter().map(prim_text).collect();
    let cw = texts.iter().map(|t| t.chars().count()).max().unwrap_or(1)
        .max(a.len().saturating_sub(1).to_string().len()).clamp(1, 12);
    let per_row = ((width.saturating_sub(1)) / (cw + 3)).max(1);
    let mut out = Vec::new();
    for start in (0..a.len()).step_by(per_row) {
        let end = (start + per_row).min(a.len());
        let idx: String = (start..end).map(|i| format!(" {:^w$}  ", i, w = cw)).collect();
        out.push(Line::from(Span::styled(idx, Style::new().fg(Color::DarkGray))));
        let seg = |l: &str, m: &str, r: &str| -> String {
            let mut s = String::from(l);
            for i in start..end {
                s.push_str(&"─".repeat(cw + 2));
                s.push_str(if i + 1 == end { r } else { m });
            }
            s
        };
        out.push(Line::from(Span::styled(seg("┌", "┬", "┐"), Style::new().dim())));
        let mut spans = vec![Span::styled("│", Style::new().dim())];
        for i in start..end {
            let t = crate::event::truncate(&texts[i], cw);
            let st = if changed.contains(&i) { Style::new().fg(Color::Black).bg(Color::Yellow).bold() } else { Style::new() };
            spans.push(Span::styled(format!(" {:^w$} ", t, w = cw), st));
            spans.push(Span::styled("│", Style::new().dim()));
        }
        out.push(Line::from(spans));
        out.push(Line::from(Span::styled(seg("└", "┴", "┘"), Style::new().dim())));
        // pointer row(s)
        let mut rows: Vec<String> = Vec::new();
        for (name, p) in ptrs {
            let p = *p;
            if p < start as i64 || p >= end as i64 {
                if !(p == end as i64 && end == a.len()) {
                    continue;
                }
            }
            let col = 1 + (p as usize - start) * (cw + 3) + cw / 2 + 1;
            let label = format!("↑{name}");
            let row = rows.iter_mut().find(|r| r.chars().count() + 1 < col);
            match row {
                Some(r) => {
                    let pad = col - r.chars().count();
                    r.push_str(&" ".repeat(pad));
                    r.push_str(&label);
                }
                None => rows.push(format!("{}{}", " ".repeat(col), label)),
            }
        }
        for r in rows {
            out.push(Line::from(Span::styled(r, Style::new().fg(Color::Cyan).bold())));
        }
    }
    out
}

fn heap_tree(a: &[Value], changed: &HashSet<usize>) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    fn walk(a: &[Value], i: usize, prefix: String, label: &str, last: bool, root: bool, ch: &HashSet<usize>, out: &mut Vec<Line<'static>>) {
        if i >= a.len() || out.len() > 200 {
            return;
        }
        let branch = if root { String::new() } else if last { format!("{prefix}└─{label} ") } else { format!("{prefix}├─{label} ") };
        let st = if ch.contains(&i) { Style::new().fg(Color::Black).bg(Color::Yellow).bold() } else { Style::new().bold() };
        out.push(Line::from(vec![Span::styled(branch, Style::new().dim()), Span::styled(prim_text(&a[i]), st), Span::styled(format!("  [{i}]"), Style::new().fg(Color::DarkGray))]));
        let np = if root { String::new() } else if last { format!("{prefix}    ") } else { format!("{prefix}│   ") };
        let (l, r) = (2 * i + 1, 2 * i + 2);
        walk(a, l, np.clone(), "L", r >= a.len(), false, ch, out);
        walk(a, r, np, "R", true, false, ch, out);
    }
    walk(a, 0, String::new(), "", true, true, changed, &mut out);
    out
}

fn matrix(a: &[Value], prev: Option<&Vec<Value>>, note: &mut String) -> Vec<Line<'static>> {
    let rows = rows_of(a).unwrap_or_default();
    let prow = prev.and_then(|p| rows_of(p));
    let cw = rows.iter().flat_map(|r| r.iter()).map(|v| prim_text(v).chars().count()).max().unwrap_or(1).clamp(1, 8);
    let ncols = rows.first().map_or(0, |r| r.len());
    let mut out = vec![Line::from(Span::styled(
        format!("    {}", (0..ncols).map(|j| format!("{:>w$} ", j, w = cw)).collect::<String>()),
        Style::new().fg(Color::DarkGray),
    ))];
    let mut changed = 0;
    for (i, r) in rows.iter().enumerate().take(60) {
        let mut spans = vec![Span::styled(format!("{:>3} ", i), Style::new().fg(Color::DarkGray))];
        for (j, v) in r.iter().enumerate() {
            let was = prow.as_ref().and_then(|p| p.get(i)).and_then(|pr| pr.get(j));
            let diff = prow.is_some() && was != Some(v);
            if diff {
                changed += 1;
            }
            let t = prim_text(v);
            let st = if diff {
                Style::new().fg(Color::Black).bg(Color::Yellow).bold()
            } else if matches!(t.as_str(), "0" | "false" | "." | "null" | "") {
                Style::new().dim()
            } else {
                Style::new()
            };
            spans.push(Span::styled(format!("{:>w$}", crate::event::truncate(&t, cw), w = cw), st));
            spans.push(Span::raw(" "));
        }
        out.push(Line::from(spans));
    }
    if changed > 0 {
        *note = format!("{changed} cells changed");
    }
    out
}

fn list(v: &Value, note: &mut String) -> Vec<Line<'static>> {
    let mut ids = HashMap::new();
    index_ids(v, &mut ids);
    let mut vals: Vec<String> = Vec::new();
    let mut seen_ids: Vec<Option<i64>> = Vec::new();
    let mut cur = v;
    let mut tail = "null".to_string();
    let mut doubly = false;
    while let Value::Object(o) = cur {
        if let Some(r) = o.get("__ref").and_then(Value::as_i64) {
            let at = seen_ids.iter().position(|x| *x == Some(r));
            tail = match at {
                Some(i) => format!("↺ cycle back to #{i}"),
                None => "↺ shared node".into(),
            };
            *note = "cycle detected".into();
            break;
        }
        if vals.len() >= 100 {
            tail = "…".into();
            break;
        }
        doubly |= o.contains_key("prev");
        vals.push(val_of(o));
        seen_ids.push(o.get("__id").and_then(Value::as_i64));
        match next_key(o).and_then(|k| o.get(k)) {
            Some(n @ Value::Object(_)) => cur = n,
            _ => break,
        }
    }
    let arrow = if doubly { " ⇄ " } else { " → " };
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, v) in vals.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(arrow, Style::new().fg(Color::Cyan)));
        }
        spans.push(Span::styled(format!("[{v}]"), Style::new().bold()));
    }
    spans.push(Span::styled(format!("{arrow}{tail}"), Style::new().fg(if tail.starts_with('↺') { Color::Red } else { Color::DarkGray })));
    if note.is_empty() {
        *note = format!("length {}{}", vals.len(), if doubly { ", doubly linked" } else { "" });
    }
    // wrap spans into lines of ~8 nodes so long lists stay readable
    let mut out = Vec::new();
    let mut line = Vec::new();
    let mut count = 0;
    for s in spans {
        let is_node = s.content.starts_with('[');
        if is_node && count == 8 {
            out.push(Line::from(std::mem::take(&mut line)));
            count = 0;
        }
        if is_node {
            count += 1;
        }
        line.push(s);
    }
    out.push(Line::from(line));
    out
}

fn tree(v: &Value, note: &mut String) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut vals: Vec<f64> = Vec::new();
    let mut size = 0usize;
    fn walk(v: &Value, prefix: &str, label: &str, last: bool, root: bool, depth: usize, out: &mut Vec<Line<'static>>, inorder: &mut Vec<f64>, size: &mut usize) -> usize {
        let Value::Object(o) = v else { return 0 };
        if out.len() > 300 {
            return 0;
        }
        let branch = if root { String::new() } else if last { format!("{prefix}└─{label} ") } else { format!("{prefix}├─{label} ") };
        if o.contains_key("__ref") {
            out.push(Line::from(vec![Span::styled(branch, Style::new().dim()), Span::styled("↺ (shared/cycle)", Style::new().fg(Color::Red))]));
            return 0;
        }
        *size += 1;
        out.push(Line::from(vec![Span::styled(branch, Style::new().dim()), Span::styled(val_of(o), Style::new().bold())]));
        let np = if root { String::new() } else if last { format!("{prefix}    ") } else { format!("{prefix}│   ") };
        let l = o.get("left").filter(|x| x.is_object());
        let r = o.get("right").filter(|x| x.is_object());
        let mut hl = 0;
        let mut hr = 0;
        if let Some(l) = l {
            hl = walk(l, &np, "L", r.is_none(), false, depth + 1, out, inorder, size);
        }
        if let Some(n) = val_num(o) {
            inorder.push(n);
        }
        if let Some(r) = r {
            hr = walk(r, &np, "R", true, false, depth + 1, out, inorder, size);
        }
        1 + hl.max(hr)
    }
    let h = walk(v, "", "", true, true, 0, &mut out, &mut vals, &mut size);
    let bst = !vals.is_empty() && vals.len() == size && vals.windows(2).all(|w| w[0] <= w[1]);
    *note = format!("size {size}, height {h}{}", if bst { ", valid BST" } else if vals.len() == size && size > 1 { ", not a BST" } else { "" });
    out
}

fn ntree(v: &Value, note: &mut String) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut size = 0;
    fn walk(v: &Value, prefix: &str, last: bool, root: bool, out: &mut Vec<Line<'static>>, size: &mut usize) -> usize {
        let Value::Object(o) = v else { return 0 };
        if out.len() > 300 {
            return 0;
        }
        *size += 1;
        let branch = if root { String::new() } else if last { format!("{prefix}└─ ") } else { format!("{prefix}├─ ") };
        let label = o.get("name").map(prim_text).unwrap_or_else(|| val_of(o));
        out.push(Line::from(vec![Span::styled(branch, Style::new().dim()), Span::styled(label, Style::new().bold())]));
        let np = if root { String::new() } else if last { format!("{prefix}   ") } else { format!("{prefix}│  ") };
        let kids = o.get("children").and_then(Value::as_array).cloned().unwrap_or_default();
        let n = kids.len();
        let mut h = 0;
        for (i, k) in kids.iter().enumerate() {
            h = h.max(walk(k, &np, i + 1 == n, false, out, size));
        }
        1 + h
    }
    let h = walk(v, "", true, true, &mut out, &mut size);
    *note = format!("size {size}, height {h}");
    out
}

fn graph(v: &Value, note: &mut String) -> Vec<Line<'static>> {
    let adj: Vec<(String, Vec<String>)> = match v {
        Value::Array(a) => a.iter().enumerate().map(|(i, r)| {
            (i.to_string(), r.as_array().map(|r| r.iter().map(prim_text).collect()).unwrap_or_default())
        }).collect(),
        Value::Object(o) => o.iter().filter(|(k, _)| !META_KEYS.contains(&k.as_str())).map(|(k, r)| {
            (k.clone(), r.as_array().map(|r| r.iter().map(prim_text).collect()).unwrap_or_default())
        }).collect(),
        _ => vec![],
    };
    let edges: usize = adj.iter().map(|(_, n)| n.len()).sum();
    let directed = adj.iter().any(|(k, ns)| ns.iter().any(|n| !adj.iter().any(|(k2, ns2)| k2 == n && ns2.contains(k))));
    *note = format!("{} nodes, {} edges{}", adj.len(), if directed { edges } else { edges / 2 }, if directed { ", directed" } else { ", undirected" });
    let kw = adj.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(1);
    adj.into_iter().take(80).map(|(k, ns)| {
        let mut s = vec![Span::styled(format!("{:>w$}", k, w = kw), Style::new().bold().fg(Color::Cyan)), Span::styled(" → ", Style::new().dim())];
        if ns.is_empty() {
            s.push(Span::styled("∅", Style::new().dim()));
        } else {
            s.push(Span::raw(ns.join(", ")));
        }
        Line::from(s)
    }).collect()
}

fn table(v: &Value, note: &mut String) -> Vec<Line<'static>> {
    let o = v.as_object().cloned().unwrap_or_default();
    let size = o.get("size").and_then(Value::as_u64).unwrap_or(0);
    if o.get("__t").and_then(Value::as_str) == Some("Set") {
        *note = format!("Set, size {size}");
        let vals = o.get("values").and_then(Value::as_array).cloned().unwrap_or_default();
        return vec![Line::from(format!("{{ {} }}", vals.iter().map(prim_text).collect::<Vec<_>>().join(", ")))];
    }
    *note = format!("Map, size {size}");
    let entries = o.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
    entries.iter().filter_map(Value::as_array).map(|p| {
        let k = p.first().map(prim_text).unwrap_or_default();
        let v = p.get(1).map(|v| if is_prim(v) { prim_text(v) } else { crate::event::compact(v, 60) }).unwrap_or_default();
        Line::from(vec![Span::styled(k, Style::new().bold().fg(Color::Cyan)), Span::styled(" ⇒ ", Style::new().dim()), Span::raw(v)])
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects() {
        assert_eq!(detect(&json!([3, 1, 2]), "arr"), Some(Kind::Array));
        assert_eq!(detect(&json!([3, 1, 2]), "minHeap"), Some(Kind::Heap));
        assert_eq!(detect(&json!([[0, 1], [1, 0]]), "g"), Some(Kind::Matrix));
        assert_eq!(detect(&json!([[1, 2], [0], [0]]), "adj"), Some(Kind::Graph));
        assert_eq!(detect(&json!({"val": 1, "next": {"val": 2, "next": null}}), "l"), Some(Kind::List));
        assert_eq!(detect(&json!({"val": 1, "left": null, "right": {"val": 2}}), "t"), Some(Kind::Tree));
        assert_eq!(detect(&json!({"arr": [1, 2, 3], "i": 0, "j": 2}), "s"), Some(Kind::Pointers));
        assert_eq!(detect(&json!({"A": ["B"], "B": ["A", "C"], "C": []}), "g"), Some(Kind::Graph));
        assert_eq!(detect(&json!({"head": {"val": 1, "next": null}}), "l"), Some(Kind::List));
        assert_eq!(detect(&json!({"name": "x", "age": 3}), "p"), None);
    }

    #[test]
    fn swap_note() {
        let v = render(&json!([1, 3, 2]), Some(&json!([1, 2, 3])), "a", 80).unwrap();
        assert_eq!(v.note, "swap [1] ↔ [2]");
    }

    #[test]
    fn cycle() {
        let v = json!({"__id": 1, "val": 1, "next": {"val": 2, "next": {"__ref": 1}}});
        assert_eq!(render(&v, None, "l", 80).unwrap().note, "cycle detected");
    }

    #[test]
    fn bst() {
        let v = json!({"val": 2, "left": {"val": 1}, "right": {"val": 3}});
        assert!(render(&v, None, "t", 80).unwrap().note.contains("valid BST"));
    }
}
