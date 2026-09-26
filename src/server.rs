use crate::event::Event;
use crate::store::{self, Store};
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::Value;
use std::{convert::Infallible, sync::Arc};
use tokio_stream::{wrappers::BroadcastStream, Stream, StreamExt};

pub const INDEX_HTML: &str = include_str!("../web/index.html");
pub const BROWSER_JS: &str = include_str!("../sdk/cdev.js");

pub fn router(store: Arc<Store>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/cdev.js", get(|| async { ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], BROWSER_JS) }))
        .route("/health", get(|| async { "ok" }))
        .route("/ingest", post(ingest).options(|| async { StatusCode::NO_CONTENT }))
        .route("/api/events", get(events))
        .route("/api/edges", get(edges))
        .route("/api/calls", get(calls))
        .route("/api/endpoints", get(endpoints))
        .route("/api/memory", get(memory))
        .route("/api/clear", post(clear))
        .route("/api/instrument", post(instrument))
        .route("/stream", get(stream))
        .route("/reload", get(reload))
        .layer(axum::middleware::map_response(cors))
        .with_state(store)
}

/// The panel page with the current snapshot inlined, so the first paint
/// already has data (no extra round trip).
async fn index(State(s): State<Arc<Store>>) -> impl IntoResponse {
    let (events, edges) = s.with(|inner| {
        let n = inner.events.len();
        let ev: Vec<Arc<Event>> = inner.events.iter().skip(n.saturating_sub(5000)).cloned().collect();
        (ev, store::edges(inner))
    });
    let json = serde_json::json!({ "events": events, "edges": edges }).to_string().replace("</", "<\\/");
    let html = INDEX_HTML.replacen("/*__CDEV_INIT__*/", &format!("window.__CDEV_INIT = {json};"), 1);
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], html)
}

async fn cors(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("*"));
    h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
    r
}

/// Accepts one event or an array, with any content-type (the browser SDK
/// posts `text/plain` to avoid CORS preflights).
async fn ingest(State(s): State<Arc<Store>>, body: Bytes) -> impl IntoResponse {
    let v: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("bad json: {e}")),
    };
    let items = match v {
        Value::Array(a) => a,
        v @ Value::Object(_) => vec![v],
        _ => return (StatusCode::BAD_REQUEST, "expected object or array".into()),
    };
    let mut n = 0;
    for item in items {
        if let Ok(e) = serde_json::from_value::<Event>(item) {
            s.push(e);
            n += 1;
        }
    }
    (StatusCode::OK, format!("{n}"))
}

#[derive(Deserialize)]
struct EvQuery {
    kind: Option<String>,
    q: Option<String>,
    limit: Option<usize>,
    since: Option<u64>,
}

async fn events(State(s): State<Arc<Store>>, Query(q): Query<EvQuery>) -> Json<Vec<Arc<Event>>> {
    let limit = q.limit.unwrap_or(2000);
    let text = q.q.unwrap_or_default();
    let since = q.since.unwrap_or(0);
    Json(s.with(|inner| {
        let mut v: Vec<Arc<Event>> = inner.events.iter().rev()
            .filter(|e| e.id > since && store::matches(e, q.kind.as_deref(), &text))
            .take(limit).cloned().collect();
        v.reverse();
        v
    }))
}

async fn edges(State(s): State<Arc<Store>>) -> Json<Vec<store::Edge>> {
    Json(s.with(store::edges))
}

#[derive(Deserialize)]
struct MemQuery {
    step: Option<usize>,
}

/// Memory at a step (default: latest): names, heap blocks, links, change note.
async fn memory(State(s): State<Arc<Store>>, Query(q): Query<MemQuery>) -> Json<crate::memory::MemView> {
    Json(s.with(|inner| crate::memory::view(inner, q.step)))
}

async fn calls(State(s): State<Arc<Store>>) -> Json<Vec<crate::api::Call>> {
    Json(s.with(|inner| {
        let mut c = crate::api::calls(inner);
        let n = c.len();
        c.drain(..n.saturating_sub(1000));
        c
    }))
}

async fn endpoints(State(s): State<Arc<Store>>) -> Json<Vec<crate::api::Endpoint>> {
    Json(s.with(crate::api::endpoints))
}

#[derive(Deserialize)]
struct InstrumentQuery {
    file: Option<String>,
}

/// Auto-watch rewrite for JS/TS (used by the Node ESM loader, Bun plugin and
/// page server). Unparseable sources come back unchanged.
async fn instrument(Query(q): Query<InstrumentQuery>, body: Bytes) -> Response {
    let src = String::from_utf8_lossy(&body).into_owned();
    let file = q.file.unwrap_or_else(|| "input.js".into());
    match crate::instrument::instrument(&src, &file) {
        Ok(out) => ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], out).into_response(),
        Err(e) => {
            let mut r = ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], src).into_response();
            if let Ok(v) = HeaderValue::from_str(&e.replace(['\n', '\r'], " ")) {
                r.headers_mut().insert(header::HeaderName::from_static("x-cdev-error"), v);
            }
            r
        }
    }
}

async fn clear(State(s): State<Arc<Store>>) -> StatusCode {
    s.clear();
    StatusCode::NO_CONTENT
}

async fn reload(State(s): State<Arc<Store>>) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let st = BroadcastStream::new(s.reload.subscribe()).filter_map(|r| r.ok().map(|m| Ok(SseEvent::default().data(m))));
    Sse::new(st).keep_alive(KeepAlive::default())
}

async fn stream(State(s): State<Arc<Store>>) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let st = BroadcastStream::new(s.tx.subscribe())
        .filter_map(|r| r.ok().and_then(|e| SseEvent::default().json_data(&*e).ok()).map(Ok));
    Sse::new(st).keep_alive(KeepAlive::default())
}
