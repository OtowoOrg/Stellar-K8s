//! Fail-open JSON-RPC proxy for Soroban state reads.
//!
//! The proxy caches only idempotent read methods. Every cache operation is
//! best-effort: a malformed key, oversized response, or cache lock failure is
//! ignored and the request is sent to the upstream RPC endpoint unchanged.

use axum::{
    body::Bytes,
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::Response,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    net::SocketAddr,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use stellar_k8s::config_reload::ConfigReloader;
use stellar_wasm_cache::{CacheConfig, StateCache};
use tokio::sync::Mutex;
use tracing::{info, warn};

const DEFAULT_LISTEN: &str = "0.0.0.0:18000";
const DEFAULT_UPSTREAM: &str = "http://127.0.0.1:8000";

#[derive(Clone)]
struct AppState {
    cache: Arc<Mutex<StateCache>>,
    client: reqwest::Client,
    upstream: String,
    upstream_requests: Arc<AtomicU64>,
}

#[derive(Debug, Deserialize)]
struct CacheFile {
    #[serde(default)]
    cache: CacheConfig,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().json().with_target(true).init();

    let listen = std::env::var("SOROBAN_CACHE_LISTEN").unwrap_or_else(|_| DEFAULT_LISTEN.into());
    let upstream = std::env::var("SOROBAN_CACHE_UPSTREAM")
        .unwrap_or_else(|_| DEFAULT_UPSTREAM.into())
        .trim_end_matches('/')
        .to_string();
    let config = load_config();
    let cache = StateCache::new(config.clone()).unwrap_or_else(|error| {
        warn!(?error, "Invalid cache configuration; using safe defaults");
        StateCache::new(CacheConfig::default()).expect("default cache configuration is valid")
    });
    let cache = Arc::new(Mutex::new(cache));

    // Hot-reload the cache config from the mounted file (#1977): a valid new
    // config swaps in a fresh cache; an invalid one is rejected and the live
    // cache keeps serving.
    let reloader = ConfigReloader::install(config, |c: &CacheConfig| {
        c.validate().map_err(|e| format!("{e:?}"))
    });
    let config_path = std::env::var("SOROBAN_CACHE_CONFIG")
        .unwrap_or_else(|_| "/config/soroban-cache.json".into());
    tokio::spawn(reloader.clone().watch_file(
        config_path,
        |bytes: &[u8]| {
            serde_json::from_slice::<CacheFile>(bytes)
                .map(|f| f.cache)
                .map_err(|e| e.to_string())
        },
        std::time::Duration::from_secs(5),
    ));
    {
        let reloader = reloader.clone();
        let cache = cache.clone();
        let mut events = reloader.subscribe();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
                let new_config = (*reloader.get_arc().await).clone();
                match StateCache::new(new_config) {
                    Ok(fresh) => {
                        *cache.lock().await = fresh;
                        info!("Soroban cache config reloaded");
                    }
                    Err(error) => {
                        warn!(?error, "Reloaded cache config unusable; keeping live cache")
                    }
                }
            }
        });
    }

    let state = AppState {
        cache,
        client: reqwest::Client::new(),
        upstream,
        upstream_requests: Arc::new(AtomicU64::new(0)),
    };
    let app = Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(health))
        .route("/stats", get(stats))
        .route("/", post(proxy))
        .with_state(state);
    let address: SocketAddr = listen.parse()?;
    info!(%address, "Starting Soroban fail-open cache proxy");
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn load_config() -> CacheConfig {
    let path = std::env::var("SOROBAN_CACHE_CONFIG")
        .unwrap_or_else(|_| "/config/soroban-cache.json".into());
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) => {
            warn!(%error, "Cache config unavailable; using defaults");
            return CacheConfig::default();
        }
    };
    match serde_json::from_str::<CacheFile>(&contents) {
        Ok(file) => file.cache,
        Err(error) => {
            warn!(%error, "Cache config invalid; using defaults");
            CacheConfig::default()
        }
    }
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn stats(State(state): State<AppState>) -> Response {
    let cache = state.cache.lock().await;
    let cache_stats = cache.stats();
    let body = serde_json::json!({
        "hits": cache_stats.hits,
        "misses": cache_stats.misses,
        "entries": cache_stats.entries,
        "storedBytes": cache_stats.stored_bytes,
        "upstreamRequests": state.upstream_requests.load(Ordering::Relaxed),
    });
    json_response(
        StatusCode::OK,
        serde_json::to_vec(&body).unwrap_or_default(),
    )
}

async fn proxy(State(state): State<AppState>, body: Bytes) -> Response {
    let request_id = request_id(&body);
    let cache_key = cache_key(&body);
    if let Some(key) = &cache_key {
        if let Some(cached) = cache_get(&state, key).await {
            return json_response(StatusCode::OK, rewrite_response_id(cached, request_id));
        }
    }

    state.upstream_requests.fetch_add(1, Ordering::Relaxed);
    let upstream_response = match state
        .client
        .post(&state.upstream)
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            warn!(%error, "Soroban upstream request failed");
            return text_response(StatusCode::BAD_GATEWAY, "upstream RPC unavailable");
        }
    };

    let status = StatusCode::from_u16(upstream_response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let response_body = match upstream_response.bytes().await {
        Ok(body) => body,
        Err(error) => {
            warn!(%error, "Failed to read Soroban upstream response");
            return text_response(
                StatusCode::BAD_GATEWAY,
                "failed to read upstream RPC response",
            );
        }
    };

    // Cache failures are deliberately ignored. The upstream result is already
    // available and must be returned regardless of cache state.
    if status.is_success() && is_cacheable_response(&response_body) {
        if let Some(key) = cache_key {
            cache_insert(&state, key, response_body.to_vec()).await;
        }
    }

    json_response(status, response_body.to_vec())
}

fn request_id(body: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(body)
        .ok()?
        .as_object()?
        .get("id")
        .cloned()
}

async fn cache_get(state: &AppState, key: &str) -> Option<Vec<u8>> {
    let mut cache = state.cache.lock().await;
    catch_unwind(AssertUnwindSafe(|| cache.get(key)))
        .ok()
        .flatten()
}

async fn cache_insert(state: &AppState, key: String, value: Vec<u8>) {
    let mut cache = state.cache.lock().await;
    let _ = catch_unwind(AssertUnwindSafe(|| cache.insert(key, value)));
}

fn cache_key(body: &[u8]) -> Option<String> {
    let request: Value = serde_json::from_slice(body).ok()?;
    let object = request.as_object()?;
    // Notifications have no response ID and must bypass the cache entirely.
    object.get("id")?;
    let method = object.get("method")?.as_str()?;
    if !matches!(
        method,
        "getLedgerEntry" | "getLedgerEntries" | "getLatestLedger" | "getNetwork"
    ) {
        return None;
    }
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    catch_unwind(AssertUnwindSafe(|| {
        StateCache::request_key(method, &params)
    }))
    .ok()
    .and_then(|result| result.ok())
}

fn is_cacheable_response(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|response| response.as_object().cloned())
        .map(|response| !response.contains_key("error"))
        .unwrap_or(false)
}

fn rewrite_response_id(body: Vec<u8>, request_id: Option<Value>) -> Vec<u8> {
    let Some(request_id) = request_id else {
        return body;
    };
    let Ok(mut response) = serde_json::from_slice::<Value>(&body) else {
        return body;
    };
    let Some(object) = response.as_object_mut() else {
        return body;
    };
    object.insert("id".to_string(), request_id);
    serde_json::to_vec(&response).unwrap_or(body)
}

fn json_response(status: StatusCode, body: Vec<u8>) -> Response {
    Response::builder()
        .status(status)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )
        .body(axum::body::Body::from(body))
        .expect("valid response")
}

fn text_response(status: StatusCode, body: &'static str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"))
        .body(axum::body::Body::from(body))
        .expect("valid response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_read_methods_get_cache_keys() {
        let read = br#"{"jsonrpc":"2.0","id":1,"method":"getLedgerEntries","params":{}}"#;
        let write = br#"{"jsonrpc":"2.0","id":1,"method":"sendTransaction","params":{}}"#;
        assert!(cache_key(read).is_some());
        let notification =
            br#"{\"jsonrpc\":\"2.0\",\"method\":\"getLedgerEntries\",\"params\":{}}"#;
        assert!(cache_key(write).is_none());
        assert!(cache_key(notification).is_none());
    }

    #[test]
    fn cached_response_uses_current_request_id() {
        let response = br#"{"jsonrpc":"2.0","id":"old","result":{"latestLedger":1}}"#;
        let rewritten = rewrite_response_id(response.to_vec(), Some(Value::from(42)));
        let value: Value = serde_json::from_slice(&rewritten).unwrap();
        assert_eq!(value.get("id"), Some(&Value::from(42)));
    }

    #[test]
    fn json_rpc_errors_are_not_cacheable() {
        let error = br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600}}"#;
        let result = br#"{"jsonrpc":"2.0","id":1,"result":{}}"#;
        assert!(!is_cacheable_response(error));
        assert!(is_cacheable_response(result));
    }
}
