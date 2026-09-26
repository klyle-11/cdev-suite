//! API view: pairs the client's and the server's record of each HTTP call,
//! and groups calls into endpoints (`GET /api/users/:id`) with status counts,
//! latency, callers and inferred body types.
//!
//! Pairing: exact by `cid` (the `x-cdev-id` header SDKs add to local
//! requests), otherwise by method + path + target + timing — the server
//! finishes a little before the client does.

use crate::event::{shape_of, Event};
use crate::store::{path_of, resolve_host, route_of, Inner};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[derive(Serialize, Clone)]
pub struct Call {
    pub id: u64,
    pub ts: f64,
    pub method: String,
    pub path: String,
    pub route: String,
    /// server service name, or external host
    pub endpoint: String,
    pub from: String,
    pub status: Option<f64>,
    pub ms: Option<f64>,
    pub err: bool,
    /// "cid" (exact), "timing" (heuristic) or null when only one side was seen
    pub paired: Option<&'static str>,
    pub client: Option<Arc<Event>>,
    pub server: Option<Arc<Event>>,
}

#[derive(Serialize)]
pub struct Endpoint {
    pub endpoint: String,
    pub method: String,
    pub route: String,
    pub count: usize,
    pub errors: usize,
    pub avg_ms: f64,
    pub max_ms: f64,
    pub statuses: BTreeMap<String, usize>,
    pub callers: Vec<String>,
    pub handlers: Vec<String>,
    /// inferred JSON shapes of request / response bodies (distinct, newest first)
    pub req_shapes: Vec<String>,
    pub res_shapes: Vec<String>,
    /// ids of the most recent calls (newest last)
    pub recent: Vec<u64>,
}

fn path_and_query(url: &str) -> String {
    match url.split_once("://") {
        Some((_, r)) => r.find('/').map(|i| r[i..].to_string()).unwrap_or_else(|| "/".into()),
        None => url.to_string(),
    }
}

fn port_of(url: &str) -> Option<String> {
    let host = url.split_once("://")?.1.split('/').next()?;
    host.rsplit_once(':').map(|(_, p)| p.to_string())
}

pub fn calls(inner: &Inner) -> Vec<Call> {
    let evs: Vec<&Arc<Event>> = inner.events.iter().filter(|e| e.kind == "http").collect();
    let ins: Vec<&Arc<Event>> = evs.iter().copied().filter(|e| e.str("dir") == "in").collect();
    let mut used = vec![false; ins.len()];
    let by_cid: HashMap<&str, usize> = ins.iter().enumerate()
        .filter(|(_, e)| !e.str("cid").is_empty()).map(|(i, e)| (e.str("cid"), i)).collect();

    let mut out: Vec<Call> = Vec::new();
    for e in evs.iter().filter(|e| e.str("dir") != "in") {
        let mut server = None;
        let mut how = None;
        if let Some(&i) = by_cid.get(e.str("cid")).filter(|_| !e.str("cid").is_empty()) {
            if !used[i] {
                used[i] = true;
                server = Some(ins[i].clone());
                how = Some("cid");
            }
        }
        if server.is_none() {
            // timing heuristic: same method + path, server is the service on that port, finished within the client's window
            let target = resolve_host(inner, e.str("url"));
            let pq = path_and_query(e.str("url"));
            let port = port_of(e.str("url"));
            let window = e.num("ms").unwrap_or(0.0) + 250.0;
            let best = ins.iter().enumerate()
                .filter(|(i, s)| !used[*i] && s.str("method") == e.str("method") && path_and_query(s.str("url")) == pq)
                .filter(|(_, s)| s.svc == target || (port.is_some() && port_of(s.str("url")) == port))
                .filter(|(_, s)| s.ts <= e.ts + 50.0 && s.ts >= e.ts - window)
                .min_by(|a, b| (e.ts - a.1.ts).abs().partial_cmp(&(e.ts - b.1.ts).abs()).unwrap());
            if let Some((i, s)) = best {
                used[i] = true;
                server = Some((*s).clone());
                how = Some("timing");
            }
        }
        out.push(make(inner, Some((*e).clone()), server, how));
    }
    for (i, s) in ins.iter().enumerate() {
        if !used[i] {
            out.push(make(inner, None, Some((*s).clone()), None));
        }
    }
    out.sort_by(|a, b| a.ts.partial_cmp(&b.ts).unwrap_or(std::cmp::Ordering::Equal));
    out
}

fn make(inner: &Inner, client: Option<Arc<Event>>, server: Option<Arc<Event>>, paired: Option<&'static str>) -> Call {
    let any = client.as_ref().or(server.as_ref()).unwrap();
    let url = any.str("url");
    let path = path_of(url);
    let endpoint = match &server {
        Some(s) => s.svc.clone(),
        None => resolve_host(inner, url),
    };
    let from = match (&client, &server) {
        (Some(c), _) => c.svc.clone(),
        (None, Some(s)) => match s.str("from") { "" => "client".into(), f => f.to_string() },
        _ => "?".into(),
    };
    // the client saw the whole round trip; prefer its status / timing
    let status = client.as_ref().and_then(|c| c.num("status")).or_else(|| server.as_ref().and_then(|s| s.num("status")));
    let ms = client.as_ref().and_then(|c| c.num("ms")).or_else(|| server.as_ref().and_then(|s| s.num("ms")));
    let err = client.as_ref().map_or(false, |c| c.get("err").is_some()) || status.map_or(false, |s| s >= 400.0);
    Call {
        id: any.id,
        ts: any.ts,
        method: any.str("method").to_string(),
        route: route_of(&path),
        path: path_and_query(url),
        endpoint,
        from,
        status,
        ms,
        err,
        paired,
        client,
        server,
    }
}

fn body_shape(side: Option<&Value>) -> Option<String> {
    let body = side?.get("body")?;
    let v = match body {
        Value::String(s) => serde_json::from_str::<Value>(s).ok()?,
        v => v.clone(),
    };
    Some(shape_of(&v))
}

pub fn endpoints(inner: &Inner) -> Vec<Endpoint> {
    let calls = calls(inner);
    let mut map: BTreeMap<(String, String, String), Endpoint> = BTreeMap::new();
    let mut ms_n: HashMap<(String, String, String), usize> = HashMap::new();
    for c in &calls {
        let key = (c.endpoint.clone(), c.method.clone(), c.route.clone());
        let ep = map.entry(key.clone()).or_insert_with(|| Endpoint {
            endpoint: c.endpoint.clone(), method: c.method.clone(), route: c.route.clone(), count: 0, errors: 0, avg_ms: 0.0, max_ms: 0.0,
            statuses: BTreeMap::new(), callers: vec![], handlers: vec![], req_shapes: vec![], res_shapes: vec![], recent: vec![],
        });
        ep.count += 1;
        if c.err {
            ep.errors += 1;
        }
        if let Some(ms) = c.ms {
            let n = ms_n.entry(key).or_default();
            ep.avg_ms = (ep.avg_ms * *n as f64 + ms) / (*n as f64 + 1.0);
            *n += 1;
            ep.max_ms = ep.max_ms.max(ms);
        }
        let st = c.status.map(|s| format!("{s}")).unwrap_or_else(|| "ERR".into());
        *ep.statuses.entry(st).or_default() += 1;
        let caller = match c.client.as_ref().map(|e| e.str("caller")) {
            Some(f) if !f.is_empty() => format!("{} · {f}", c.from),
            _ => c.from.clone(),
        };
        if !ep.callers.contains(&caller) {
            ep.callers.push(caller);
        }
        if let Some(h) = c.server.as_ref().map(|s| s.str("handler")).filter(|h| !h.is_empty()) {
            let h = match h.split_once(' ') {
                Some((m, p)) if p.starts_with('/') => format!("{m} {}", route_of(p)),
                _ => h.to_string(),
            };
            if !ep.handlers.contains(&h) {
                ep.handlers.push(h);
            }
        }
        // body types: from whichever side recorded a body
        let req = c.client.as_ref().and_then(|e| body_shape(e.get("req"))).or_else(|| c.server.as_ref().and_then(|e| body_shape(e.get("req"))));
        let res = c.server.as_ref().and_then(|e| body_shape(e.get("res"))).or_else(|| c.client.as_ref().and_then(|e| body_shape(e.get("res"))));
        for (shape, list) in [(req, &mut ep.req_shapes), (res, &mut ep.res_shapes)] {
            if let Some(s) = shape {
                list.retain(|x| x != &s);
                list.insert(0, s);
                list.truncate(3);
            }
        }
        ep.recent.push(c.id);
        if ep.recent.len() > 20 {
            ep.recent.remove(0);
        }
    }
    let mut v: Vec<Endpoint> = map.into_values().collect();
    v.sort_by(|a, b| a.endpoint.cmp(&b.endpoint).then(a.route.cmp(&b.route)).then(a.method.cmp(&b.method)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use serde_json::json;

    fn push(st: &Store, v: Value) {
        st.push(serde_json::from_value(v).unwrap());
    }

    #[test]
    fn pairs_and_groups() {
        let st = Store::new(100);
        push(&st, json!({"kind":"service","svc":"api","name":"api","port":3000}));
        // exact pair via cid
        push(&st, json!({"kind":"http","svc":"api","dir":"in","method":"GET","url":"http://localhost:3000/u/1","cid":"abc","status":200,"ms":2.0,"ts":1000.0,
            "res":{"body":"{\"id\":1,\"name\":\"a\"}"}}));
        push(&st, json!({"kind":"http","svc":"web","dir":"out","method":"GET","url":"http://localhost:3000/u/1","cid":"abc","status":200,"ms":5.0,"ts":1003.0,"caller":"load"}));
        // timing pair (browser cross-origin: no cid)
        push(&st, json!({"kind":"http","svc":"api","dir":"in","method":"GET","url":"http://localhost:3000/u/2","status":404,"ms":1.0,"ts":2000.0}));
        push(&st, json!({"kind":"http","svc":"browser","dir":"out","method":"GET","url":"http://localhost:3000/u/2","status":404,"ms":4.0,"ts":2002.0}));
        // client only (external)
        push(&st, json!({"kind":"http","svc":"api","dir":"out","method":"POST","url":"https://example.com/x","status":201,"ms":40.0,"ts":3000.0}));
        let (calls, eps) = st.with(|i| (calls(i), endpoints(i)));
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].paired, Some("cid"));
        assert_eq!(calls[1].paired, Some("timing"));
        assert_eq!(calls[2].paired, None);
        let users = eps.iter().find(|e| e.route == "/u/:id").unwrap();
        assert_eq!(users.count, 2);
        assert_eq!(users.errors, 1);
        assert_eq!(users.res_shapes[0], "{ id: number, name: string }");
        assert!(users.callers.contains(&"web · load".to_string()));
    }
}
