//! Memory view: rebuilds "what memory looks like at step N" from the `mem`
//! field SDKs attach to watch events.
//!
//! * names   — watched variables (stack/static slots for C++/Rust/Go, name
//!             bindings for Python/JS) with address, size, type
//! * blocks  — heap things: buffers (Vec/vector/String data), pointer targets,
//!             objects (Python/JS), linked by pointers, items and fields
//! * a note describing what the step changed (reallocation, move, repoint…)

use crate::event::{compact, Event};
use crate::store::Inner;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[derive(Serialize, Default, Clone)]
pub struct MemName {
    pub svc: String,
    pub name: String,
    pub ty: String,
    pub lang: String,
    pub addr: Option<String>,
    pub size: Option<u64>,
    pub region: String,
    pub value: String,
    /// block address this name points at (pointer target, buffer, or its own object)
    pub to: Option<String>,
    /// another watched name this points at (pointer to a stack variable)
    pub to_name: Option<String>,
    pub refs: Option<u64>,
    pub changed: bool,
    /// (block addr, byte offset) when this name lives inside a heap block (e.g. a node's field)
    pub inside: Option<(String, u64)>,
}

#[derive(Serialize, Default, Clone)]
pub struct Block {
    pub addr: String,
    /// "buffer" | "target" | "object"
    pub kind: String,
    pub size: Option<u64>,
    pub owners: Vec<String>,
    pub len: Option<u64>,
    pub cap: Option<u64>,
    pub elem: Option<u64>,
    /// "heap" | "inline" (small-string) | "stack" | "static" | "none" (no allocation yet)
    pub region: String,
    pub cells: Vec<String>,
    /// (label, target block addr): list items / object fields that are references
    pub links: Vec<(String, String)>,
    /// how many names/links point here (>1 = shared / aliased)
    pub shared: usize,
    pub changed: bool,
    /// (block addr, byte offset) when this block sits inside another block
    pub inside: Option<(String, u64)>,
}

#[derive(Serialize, Default)]
pub struct MemView {
    pub steps: usize,
    pub step: usize,
    /// source location of the watch that produced this step
    pub loc: Option<String>,
    pub changed: Option<String>,
    pub note: String,
    pub names: Vec<MemName>,
    pub blocks: Vec<Block>,
}

fn mem_events(inner: &Inner) -> Vec<Arc<Event>> {
    inner.events.iter().filter(|e| e.kind == "watch" && e.get("mem").map_or(false, Value::is_object)).cloned().collect()
}

fn s(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_string)
}
fn n(v: Option<&Value>) -> Option<u64> {
    v.and_then(Value::as_u64)
}

pub fn view(inner: &Inner, step: Option<usize>) -> MemView {
    let evs = mem_events(inner);
    if evs.is_empty() {
        return MemView::default();
    }
    let step = step.unwrap_or(evs.len() - 1).min(evs.len() - 1);
    // latest snapshot of each name up to `step`, plus the one before it (for the note)
    let mut latest: BTreeMap<(String, String), (usize, Arc<Event>)> = BTreeMap::new();
    let mut prev_of_changed: Option<Arc<Event>> = None;
    let cur = &evs[step];
    for (i, e) in evs.iter().enumerate().take(step + 1) {
        let key = (e.svc.clone(), e.str("name").to_string());
        if i == step {
            prev_of_changed = latest.get(&key).map(|(_, p)| p.clone());
        }
        latest.insert(key, (i, e.clone()));
    }
    let mut order: Vec<(usize, Arc<Event>)> = latest.into_values().collect();
    order.sort_by_key(|(i, _)| *i);

    let mut names = Vec::new();
    let mut blocks: HashMap<String, Block> = HashMap::new();
    let mut by_addr: HashMap<String, String> = HashMap::new(); // stack/static addr → name
    for (_, e) in &order {
        let m = e.get("mem").unwrap();
        if let Some(a) = s(m.get("addr")) {
            if matches!(m.get("region").and_then(Value::as_str), Some("stack" | "static")) {
                by_addr.insert(a, e.str("name").to_string());
            }
        }
    }

    for (_, e) in &order {
        let m = e.get("mem").unwrap();
        let name = e.str("name").to_string();
        let region = s(m.get("region")).unwrap_or_else(|| "value".into());
        let addr = s(m.get("addr"));
        let v = e.get("v").unwrap_or(&Value::Null);
        let changed = Arc::ptr_eq(e, cur);
        let mut nm = MemName {
            svc: e.svc.clone(),
            name: name.clone(),
            ty: match e.str("t") { "" => e.str("shape").to_string(), t => t.to_string() },
            lang: e.str("lang").to_string(),
            addr: addr.clone(),
            size: n(m.get("size")),
            region: region.clone(),
            value: compact(v, 60),
            refs: n(m.get("refs")),
            changed,
            ..Default::default()
        };

        // pointer / smart pointer / reference target
        if let Some(p) = s(m.get("ptr")) {
            if let Some(other) = by_addr.get(&p).filter(|o| **o != name) {
                nm.to_name = Some(other.clone());
            } else {
                let b = block_for(&mut blocks, &p, "target", format!("*{name}"), changed);
                b.size = n(m.get("target_size")).or(b.size);
                b.region = s(m.get("target_region")).unwrap_or_else(|| "heap".into());
                // raw pointers serialize as their own address; that isn't the target's content
                let is_addr = v.as_str().map_or(false, |t| t.starts_with("0x"));
                if b.cells.is_empty() && !v.is_null() && !is_addr {
                    b.cells = vec![compact(v, 40)];
                }
                nm.to = Some(p);
            }
        }
        // owned buffer (Vec / vector / String data)
        if let Some(h) = m.get("heap").filter(|h| h.is_object()) {
            let ha = s(h.get("addr")).unwrap_or_default();
            let hr = s(h.get("region")).unwrap_or_else(|| "heap".into());
            if hr != "none" {
                let len = n(h.get("len"));
                let cap = n(h.get("cap"));
                let elem = n(h.get("elem"));
                let cells: Vec<String> = match v {
                    Value::Array(a) => a.iter().map(|x| compact(x, 12)).collect(),
                    Value::String(t) => t.chars().map(|c| c.to_string()).collect(),
                    Value::Object(o) => o.values().find_map(|x| x.as_array()).map(|a| a.iter().map(|x| compact(x, 12)).collect()).unwrap_or_default(),
                    _ => vec![],
                };
                let b = block_for(&mut blocks, &ha, "buffer", format!("{name}'s buffer"), changed);
                b.len = len;
                b.cap = cap;
                b.elem = elem;
                b.size = match (cap, elem) { (Some(c), Some(e)) => Some(c * e), _ => None };
                b.region = hr;
                b.cells = cells;
                nm.to = Some(ha);
            }
        }
        // the value itself lives in the heap (Python/JS objects). A heap pointer
        // (C++ `head->next`) is shown by what it points at + where it sits instead.
        if region == "heap" && m.get("ptr").is_none() && m.get("heap").is_none() {
            if let Some(a) = &addr {
                let b = block_for(&mut blocks, a, "object", name.clone(), changed);
                b.size = nm.size;
                b.cells = vec![compact(v, 40)];
                if let Some(Value::Array(items)) = m.get("items") {
                    b.links = items.iter().enumerate().filter_map(|(i, x)| x.as_str().map(|a| (format!("[{i}]"), a.to_string()))).collect();
                }
                if let Some(Value::Object(f)) = m.get("fields") {
                    b.links = f.iter().filter_map(|(k, x)| x.as_str().map(|a| (format!(".{k}"), a.to_string()))).collect();
                }
                nm.to = Some(a.clone());
            }
        }
        names.push(nm);
    }

    // make blocks for link targets and count how often each is referenced
    let links: Vec<(String, String, String)> = blocks.values()
        .flat_map(|b| b.links.iter().map(|(l, t)| (b.owners.first().cloned().unwrap_or_default(), l.clone(), t.clone())))
        .collect();
    for (owner, label, target) in links {
        let b = blocks.entry(target.clone()).or_insert_with(|| Block { addr: target.clone(), kind: "object".into(), region: "heap".into(), ..Default::default() });
        let o = format!("{owner}{label}");
        if !b.owners.contains(&o) {
            b.owners.push(o);
        }
        b.shared += 1;
    }
    // element blocks for Python/JS containers: show the element's value when we have it
    for (_, e) in &order {
        let m = e.get("mem").unwrap();
        if let (Some(Value::Array(items)), Some(Value::Array(vals))) = (m.get("items"), e.get("v")) {
            for (id, val) in items.iter().zip(vals) {
                if let Some(b) = id.as_str().and_then(|a| blocks.get_mut(a)) {
                    if b.cells.is_empty() {
                        b.cells = vec![compact(val, 40)];
                    }
                }
            }
        }
    }

    let mut blocks: Vec<Block> = blocks.into_values().collect();
    blocks.sort_by(|a, b| addr_key(&a.addr).cmp(&addr_key(&b.addr)));

    // containment: a node's field lives inside the node's block
    let spans: Vec<(String, u64, u64)> = blocks.iter()
        .filter_map(|b| Some((b.addr.clone(), num(&b.addr)?, b.size?)))
        .filter(|(_, _, sz)| *sz > 0).collect();
    let container = |addr: &str| -> Option<(String, u64)> {
        let a = num(addr)?;
        spans.iter().filter(|(ba, start, sz)| ba != addr && a >= *start && a < start + sz)
            .map(|(ba, start, _)| (ba.clone(), a - start)).next()
    };
    for b in blocks.iter_mut() {
        b.inside = container(&b.addr);
    }
    for nm in names.iter_mut() {
        if nm.region == "heap" {
            nm.inside = nm.addr.as_deref().and_then(|a| container(a));
        }
    }

    let mut note = note(prev_of_changed.as_deref(), cur);
    // two names, one object (Python `b = a`, JS `const b = a`)
    let cur_name = cur.str("name");
    let cur_addr = cur.get("mem").and_then(|m| m.get("addr")).and_then(Value::as_str);
    if let Some(addr) = cur_addr {
        let same: Vec<&str> = names.iter()
            .filter(|n| n.name != cur_name && n.region == "heap" && n.addr.as_deref() == Some(addr))
            .map(|n| n.name.as_str()).collect();
        if !same.is_empty() {
            note += &format!(" · same object as {}", same.join(", "));
        }
    }

    MemView {
        steps: evs.len(),
        step,
        loc: Some(cur.str("loc").to_string()).filter(|l| !l.is_empty()),
        changed: Some(cur_name.to_string()),
        note,
        names,
        blocks,
    }
}

/// Get or create the block at `addr`, recording one more reference from `owner`.
fn block_for<'a>(blocks: &'a mut HashMap<String, Block>, addr: &str, kind: &str, owner: String, changed: bool) -> &'a mut Block {
    let b = blocks.entry(addr.to_string()).or_insert_with(|| Block { addr: addr.to_string(), kind: kind.into(), region: "heap".into(), ..Default::default() });
    if !b.owners.contains(&owner) {
        b.owners.push(owner);
    }
    b.shared += 1;
    b.changed |= changed;
    b
}

fn num(a: &str) -> Option<u64> {
    a.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok())
}

fn addr_key(a: &str) -> (u8, u64) {
    match a.strip_prefix("0x").and_then(|h| u64::from_str_radix(h, 16).ok()) {
        Some(x) => (0, x),
        None => (1, a.trim_start_matches('#').parse().unwrap_or(0)),
    }
}

/// What changed in memory between two snapshots of the same variable.
pub fn note(prev: Option<&Event>, cur: &Event) -> String {
    let name = cur.str("name");
    let m = cur.get("mem").cloned().unwrap_or(Value::Null);
    let mut out: Vec<String> = Vec::new();
    let Some(p) = prev.and_then(|p| p.get("mem")) else {
        let mut first = format!("{name}: first seen");
        if let Some(a) = m.get("addr").and_then(Value::as_str) {
            first += &format!(" at {a}");
        }
        if let Some(r) = m.get("region").and_then(Value::as_str) {
            first += &format!(" ({r})");
        }
        if let Some(a) = alias_note(&m) {
            first += &format!(" · {a}");
        }
        return first;
    };
    let g = |v: &Value, k: &str| v.get(k).cloned().unwrap_or(Value::Null);
    let (pa, ca) = (g(p, "addr"), g(&m, "addr"));
    if pa != ca && !pa.is_null() && !ca.is_null() {
        out.push(format!("{name} moved {} → {}", pa.as_str().unwrap_or("?"), ca.as_str().unwrap_or("?")));
    }
    let (ph, ch) = (g(p, "heap"), g(&m, "heap"));
    if ch.is_object() {
        let (pr, cr) = (ph.get("region").and_then(Value::as_str).unwrap_or("none"), ch.get("region").and_then(Value::as_str).unwrap_or("none"));
        let (pad, cad) = (g(&ph, "addr"), g(&ch, "addr"));
        let (pc, cc) = (g(&ph, "cap"), g(&ch, "cap"));
        if pr == "none" && cr != "none" {
            out.push(format!("{name}: first allocation at {} (cap {cc})", cad.as_str().unwrap_or("?")));
        } else if pr == "inline" && cr == "heap" {
            out.push(format!("{name}: outgrew its inline buffer → heap {} (cap {cc})", cad.as_str().unwrap_or("?")));
        } else if pad != cad {
            out.push(format!("{name}: buffer reallocated {} → {} (cap {pc} → {cc})", pad.as_str().unwrap_or("?"), cad.as_str().unwrap_or("?")));
        } else if pc != cc {
            out.push(format!("{name}: buffer grew in place (cap {pc} → {cc})"));
        }
        let (pl, cl) = (g(&ph, "len"), g(&ch, "len"));
        if out.is_empty() && pl != cl {
            out.push(format!("{name}: len {pl} → {cl} (cap {cc}, no reallocation)"));
        }
    }
    let (pp, cp) = (g(p, "ptr"), g(&m, "ptr"));
    if pp != cp {
        out.push(match cp.as_str() {
            Some(a) => format!("{name} now points at {a}"),
            None => format!("{name} is now null"),
        });
    }
    let (pr, cr) = (g(p, "refs"), g(&m, "refs"));
    if pr != cr && !cr.is_null() {
        out.push(format!("{name}: {pr} → {cr} references"));
    }
    if out.is_empty() {
        out.push(format!("{name}: value changed, same memory"));
    }
    if let Some(a) = alias_note(&m) {
        out.push(a);
    }
    out.join(" · ")
}

/// "⚠ [0], [1], [2] are the same object" when a container holds one object several times.
fn alias_note(m: &Value) -> Option<String> {
    let items = m.get("items")?.as_array()?;
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, x) in items.iter().enumerate() {
        if let Some(a) = x.as_str() {
            groups.entry(a).or_default().push(i);
        }
    }
    let mut dup: Vec<Vec<usize>> = groups.into_values().filter(|g| g.len() > 1).collect();
    dup.sort();
    let g = dup.first()?;
    let idx: Vec<String> = g.iter().map(|i| format!("[{i}]")).collect();
    Some(format!("⚠ {} are the same object — changing one changes all", idx.join(", ")))
}

/// Items that are the same object (Python `[[0]*3]*3`, JS shared refs).
#[cfg(test)]
pub fn aliasing(b: &Block) -> Option<String> {
    (b.shared > 1 && b.kind == "object").then(|| format!("shared by {}", b.owners.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use serde_json::json;

    fn push(store: &Store, v: serde_json::Value) {
        store.push(serde_json::from_value(v).unwrap());
    }

    #[test]
    fn realloc_and_alias() {
        let st = Store::new(100);
        push(&st, json!({"kind":"watch","svc":"c","name":"v","v":[1],"mem":{"addr":"0x10","size":24,"region":"stack","heap":{"addr":"0xa0","len":1,"cap":1,"elem":4,"region":"heap"}}}));
        push(&st, json!({"kind":"watch","svc":"c","name":"v","v":[1,2],"mem":{"addr":"0x10","size":24,"region":"stack","heap":{"addr":"0xb0","len":2,"cap":2,"elem":4,"region":"heap"}}}));
        push(&st, json!({"kind":"watch","svc":"c","name":"p","v":"0x10","mem":{"addr":"0x20","size":8,"region":"stack","ptr":"0x10"}}));
        push(&st, json!({"kind":"watch","svc":"py","name":"grid","v":[[0],[0]],"mem":{"addr":"0x90","size":72,"region":"heap","items":["0x91","0x91"]}}));
        let mv = st.with(|i| view(i, Some(1)));
        assert_eq!(mv.note, "v: buffer reallocated 0xa0 → 0xb0 (cap 1 → 2)");
        let mv = st.with(|i| view(i, None));
        let p = mv.names.iter().find(|n| n.name == "p").unwrap();
        assert_eq!(p.to_name.as_deref(), Some("v")); // pointer to a stack variable links to the name
        push(&st, json!({"kind":"watch","svc":"c","name":"head","v":"0x100","mem":{"addr":"0x30","size":8,"region":"stack","ptr":"0x100","target_size":16}}));
        push(&st, json!({"kind":"watch","svc":"c","name":"head->next","v":"0x200","mem":{"addr":"0x108","size":8,"region":"heap","ptr":"0x200","target_size":16}}));
        let mv = st.with(|i| view(i, None));
        let next = mv.names.iter().find(|n| n.name == "head->next").unwrap();
        assert_eq!(next.inside, Some(("0x100".to_string(), 8)));
        assert_eq!(next.to.as_deref(), Some("0x200"));
        let row = mv.blocks.iter().find(|b| b.addr == "0x91").unwrap();
        assert_eq!(row.shared, 2);
        assert!(aliasing(row).is_some());
    }
}
