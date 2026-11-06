//! The HTTP API and the dashboard it serves (`--features server`).
//!
//! Everything under `/api` speaks JSON, and every failure has the same shape:
//! `{"error": {"code": "...", "message": "...", "details": {...}}}`.
//!
//! The store usually belongs to another process, so the server is read-only unless
//! asked otherwise. It binds to loopback by default, where it needs no token, so it
//! refuses the two ways a web page could reach it anyway: DNS rebinding (the Host
//! check) and cross-site form posts (POST must be JSON, which forces a preflight this
//! server never approves).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::{DbError, Entry};
use crate::db::Database;
use crate::projection;

const MAX_BODY_BYTES: usize = 512 * 1024 * 1024;
const MAX_BATCH: usize = 100_000;
const MAX_ID_CHARS: usize = 1024;
const MAX_K: usize = 1000;
const MAX_PAGE: usize = 1000;
const MAX_PROJECTION: usize = 10_000;

const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
     img-src 'self' data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

macro_rules! asset {
    ($path:literal, $mime:literal) => {
        ($path, $mime, include_str!(concat!("../dashboard/", $path)))
    };
}

const ASSETS: &[(&str, &str, &str)] = &[
    asset!("style.css", "text/css; charset=utf-8"),
    asset!("theme.js", "text/javascript; charset=utf-8"),
    asset!("mark.svg", "image/svg+xml"),
    asset!("app.js", "text/javascript; charset=utf-8"),
    asset!("api.js", "text/javascript; charset=utf-8"),
    asset!("ui.js", "text/javascript; charset=utf-8"),
    asset!("palette.js", "text/javascript; charset=utf-8"),
    asset!("pages/overview.js", "text/javascript; charset=utf-8"),
    asset!("pages/explore.js", "text/javascript; charset=utf-8"),
    asset!("pages/scatter.js", "text/javascript; charset=utf-8"),
    asset!("pages/vector.js", "text/javascript; charset=utf-8"),
    asset!("pages/search.js", "text/javascript; charset=utf-8"),
    asset!("pages/browse.js", "text/javascript; charset=utf-8"),
    asset!("pages/import.js", "text/javascript; charset=utf-8"),
    asset!("pages/connect.js", "text/javascript; charset=utf-8"),
];
const INDEX_HTML: &str = include_str!("../dashboard/index.html");

pub struct Config {
    pub data: PathBuf,
    pub writable: bool,
    /// When set, every `/api` route except health needs `Authorization: Bearer <token>`.
    pub token: Option<String>,
    /// Bound to loopback: only accept requests addressed to a loopback name.
    pub loopback_only: bool,
}

struct Shared {
    db: RwLock<Database>,
    config: Config,
    started: Instant,
    queries: AtomicU64,
    query_micros: AtomicU64,
}

type AppState = Arc<Shared>;

/// Open the store and build the router.
pub fn app(config: Config) -> Result<Router, DbError> {
    let db = if config.writable {
        Database::open(&config.data)?
    } else {
        Database::open_read_only(&config.data)?
    };
    let state: AppState = Arc::new(Shared {
        db: RwLock::new(db),
        config,
        started: Instant::now(),
        queries: AtomicU64::new(0),
        query_micros: AtomicU64::new(0),
    });
    Ok(Router::new()
        .route("/", get(index))
        .route("/assets/*path", get(serve_asset))
        .route("/api/health", get(health))
        .route("/api/stats", get(stats))
        .route("/api/vectors", get(list_vectors).post(add_vectors))
        .route("/api/vectors/:id", get(get_vector).delete(delete_vector))
        .route("/api/search", post(search))
        .route("/api/projection", get(projection))
        .route("/api/snapshots", post(create_snapshot))
        .route("/api/reload", post(reload))
        .fallback(not_found)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state))
}

// ---- errors ---------------------------------------------------------------------------------

struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    details: Option<Value>,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), details: None }
    }

    fn invalid(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, code, message)
    }

    fn internal() -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL", "Something went wrong on the server.")
    }

    fn read_only() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "READ_ONLY",
            "This server is read-only. Restart it with --writable to change data.",
        )
    }

    fn not_found(id: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, "NOT_FOUND", format!("No vector with id \"{id}\"."))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut error = json!({ "code": self.code, "message": self.message });
        if let Some(details) = self.details {
            error["details"] = details;
        }
        (self.status, Json(json!({ "error": error }))).into_response()
    }
}

impl From<DbError> for ApiError {
    fn from(e: DbError) -> Self {
        match e {
            DbError::Dimension { expected, got } => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                code: "DIMENSION_MISMATCH",
                message: format!("This store holds {expected}-dimensional vectors, and this one has {got}."),
                details: Some(json!({ "expected": expected, "got": got })),
            },
            DbError::ReadOnly => Self::read_only(),
            other => {
                eprintln!("athenadb: {other}");
                Self::internal()
            }
        }
    }
}

impl From<JsonRejection> for ApiError {
    fn from(e: JsonRejection) -> Self {
        Self::new(e.status(), "INVALID_JSON", e.body_text())
    }
}

impl From<QueryRejection> for ApiError {
    fn from(e: QueryRejection) -> Self {
        Self::new(e.status(), "INVALID_QUERY", e.body_text())
    }
}

type ApiResult<T> = Result<T, ApiError>;

/// Run against the store off the async runtime: searches are CPU-bound and writes fsync.
async fn reading<T: Send + 'static>(
    state: &AppState,
    f: impl FnOnce(&Database) -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    let state = state.clone();
    tokio::task::spawn_blocking(move || f(&*state.db.read().map_err(|_| ApiError::internal())?))
        .await
        .map_err(|_| ApiError::internal())?
}

async fn writing<T: Send + 'static>(
    state: &AppState,
    f: impl FnOnce(&mut Database) -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    let state = state.clone();
    tokio::task::spawn_blocking(move || f(&mut *state.db.write().map_err(|_| ApiError::internal())?))
        .await
        .map_err(|_| ApiError::internal())?
}

// ---- shared helpers -------------------------------------------------------------------------

/// A query string's `namespace=` means no filter, not the empty namespace.
fn filter(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|s| !s.is_empty())
}

fn matches(e: &Entry, namespace: Option<&str>, model: Option<&str>) -> bool {
    namespace.is_none_or(|ns| e.namespace == ns) && model.is_none_or(|m| e.embed_model_id == m)
}

fn in_range(name: &str, value: usize, max: usize) -> ApiResult<usize> {
    if value == 0 || value > max {
        return Err(ApiError::invalid("OUT_OF_RANGE", format!("{name} must be between 1 and {max}.")));
    }
    Ok(value)
}

// ---- middleware -----------------------------------------------------------------------------

fn is_loopback_host(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host.split(':').next().unwrap_or("")
    };
    matches!(name, "localhost" | "127.0.0.1" | "::1")
}

fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    let is_api = path.starts_with("/api/");

    if state.config.loopback_only {
        if let Some(host) = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()) {
            if !is_loopback_host(host) {
                return ApiError::new(StatusCode::FORBIDDEN, "FORBIDDEN_HOST", "This server only answers to localhost.")
                    .into_response();
            }
        }
    }

    if is_api && req.method() == Method::POST {
        let is_json = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|c| c.to_str().ok())
            .is_some_and(|c| c.starts_with("application/json"));
        if !is_json {
            return ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "JSON_REQUIRED",
                "POST requests must send Content-Type: application/json.",
            )
            .into_response();
        }
    }

    if let Some(token) = &state.config.token {
        if is_api && path != "/api/health" {
            let supplied = req
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|a| a.to_str().ok())
                .and_then(|a| a.strip_prefix("Bearer "));
            if !supplied.is_some_and(|s| same_secret(s, token)) {
                return ApiError::new(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "A valid token is required.")
                    .into_response();
            }
        }
    }

    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CONTENT_SECURITY_POLICY));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

// ---- dashboard ------------------------------------------------------------------------------

async fn index() -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], INDEX_HTML)
        .into_response()
}

async fn serve_asset(Path(path): Path<String>) -> Response {
    match ASSETS.iter().find(|(p, _, _)| *p == path) {
        Some((_, mime, body)) => {
            ([(header::CONTENT_TYPE, *mime), (header::CACHE_CONTROL, "no-cache")], *body).into_response()
        }
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

async fn not_found(req: Request) -> Response {
    if req.uri().path().starts_with("/api/") {
        ApiError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "No such endpoint.").into_response()
    } else {
        (StatusCode::NOT_FOUND, "Not found").into_response()
    }
}

// ---- status ---------------------------------------------------------------------------------

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

async fn stats(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let shared = state.clone();
    reading(&state, move |db| {
        let s = db.stats();
        let queries = shared.queries.load(Ordering::Relaxed);
        let micros = shared.query_micros.load(Ordering::Relaxed);
        Ok(Json(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "read_only": s.read_only,
            "path": std::fs::canonicalize(db.path()).unwrap_or_else(|_| db.path().to_path_buf()),
            "uptime_seconds": shared.started.elapsed().as_secs(),
            "vectors": s.vectors,
            "dimensions": s.dimensions,
            "zero_norm_vectors": s.zero_norm_vectors,
            "groups": s.groups.iter().map(|g| json!({
                "namespace": g.namespace,
                "embed_model_id": g.embed_model_id,
                "vectors": g.vectors,
            })).collect::<Vec<_>>(),
            "durability": {
                "wal_bytes": s.disk.wal_bytes,
                "snapshot_bytes": s.disk.snapshot_bytes,
                "snapshot_modified_unix": s.disk.snapshot_modified_unix,
                "ops_since_snapshot": s.ops_since_snapshot,
                "snapshot_interval": s.snapshot_interval,
                "disk_changed": s.disk_changed,
            },
            "queries": {
                "count": queries,
                "mean_micros": micros.checked_div(queries).unwrap_or(0),
            },
        })))
    })
    .await
}

// ---- vectors --------------------------------------------------------------------------------

#[derive(Deserialize)]
struct ListParams {
    namespace: Option<String>,
    embed_model_id: Option<String>,
    /// Case-insensitive substring of the id.
    q: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

#[derive(Serialize)]
struct VectorSummary<'a> {
    id: &'a str,
    namespace: &'a str,
    embed_model_id: &'a str,
    metadata: &'a Value,
}

async fn list_vectors(
    State(state): State<AppState>,
    params: Result<Query<ListParams>, QueryRejection>,
) -> ApiResult<Json<Value>> {
    let Query(params) = params?;
    let limit = in_range("limit", params.limit.unwrap_or(50), MAX_PAGE)?;
    let offset = params.offset.unwrap_or(0);
    reading(&state, move |db| {
        let namespace = filter(&params.namespace);
        let model = filter(&params.embed_model_id);
        let needle = filter(&params.q).map(str::to_lowercase);
        let mut total = 0usize;
        let mut page = Vec::new();
        for e in db.entries() {
            if !matches(e, namespace, model) {
                continue;
            }
            if needle.as_ref().is_some_and(|n| !e.id.to_lowercase().contains(n)) {
                continue;
            }
            if total >= offset && page.len() < limit {
                page.push(VectorSummary {
                    id: &e.id,
                    namespace: &e.namespace,
                    embed_model_id: &e.embed_model_id,
                    metadata: &e.metadata,
                });
            }
            total += 1;
        }
        Ok(Json(json!({
            "data": page,
            "pagination": { "limit": limit, "offset": offset, "total": total },
        })))
    })
    .await
}

async fn get_vector(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    reading(&state, move |db| {
        let e = db.get(&id).ok_or_else(|| ApiError::not_found(&id))?;
        Ok(Json(json!({
            "id": e.id,
            "namespace": e.namespace,
            "embed_model_id": e.embed_model_id,
            "metadata": e.metadata,
            "vector": e.vector,
        })))
    })
    .await
}

#[derive(Deserialize)]
struct NewVector {
    id: String,
    vector: Vec<f32>,
    #[serde(default)]
    metadata: Value,
    namespace: Option<String>,
    embed_model_id: Option<String>,
}

#[derive(Deserialize)]
struct AddBody {
    vectors: Vec<NewVector>,
}

async fn add_vectors(
    State(state): State<AppState>,
    body: Result<Json<AddBody>, JsonRejection>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if !state.config.writable {
        return Err(ApiError::read_only());
    }
    let Json(body) = body?;
    if body.vectors.is_empty() {
        return Err(ApiError::invalid("EMPTY_BATCH", "Send at least one vector."));
    }
    if body.vectors.len() > MAX_BATCH {
        return Err(ApiError::invalid(
            "BATCH_TOO_LARGE",
            format!("Send at most {MAX_BATCH} vectors per request."),
        ));
    }
    let mut entries = Vec::with_capacity(body.vectors.len());
    for (i, v) in body.vectors.into_iter().enumerate() {
        if v.id.is_empty() || v.id.chars().count() > MAX_ID_CHARS {
            return Err(ApiError::invalid(
                "INVALID_ID",
                format!("Vector {i}: an id must be 1 to {MAX_ID_CHARS} characters."),
            ));
        }
        if v.vector.is_empty() {
            return Err(ApiError::invalid("INVALID_VECTOR", format!("Vector {i} (\"{}\") is empty.", v.id)));
        }
        if v.vector.iter().any(|x| !x.is_finite()) {
            return Err(ApiError::invalid(
                "INVALID_VECTOR",
                format!("Vector {i} (\"{}\") has a value too large for a 32-bit float.", v.id),
            ));
        }
        entries.push(Entry {
            id: v.id,
            vector: v.vector,
            metadata: v.metadata,
            namespace: v.namespace.filter(|n| !n.is_empty()).unwrap_or_else(|| "default".to_string()),
            embed_model_id: v.embed_model_id.unwrap_or_default(),
            inv_norm: 0.0,
        });
    }
    let added = entries.len();
    let total = writing(&state, move |db| {
        db.add_batch(entries)?;
        Ok(db.len())
    })
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "added": added, "vectors": total }))))
}

async fn delete_vector(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    if !state.config.writable {
        return Err(ApiError::read_only());
    }
    let deleted = writing(&state, move |db| Ok(db.delete(&id)?)).await?;
    Ok(Json(json!({ "deleted": deleted })))
}

// ---- search ---------------------------------------------------------------------------------

#[derive(Deserialize)]
struct SearchBody {
    /// Search by a stored vector's id; the vector itself is left out of the results.
    id: Option<String>,
    /// Or search by a vector you supply.
    vector: Option<Vec<f32>>,
    k: Option<usize>,
    namespace: Option<String>,
    embed_model_id: Option<String>,
}

async fn search(
    State(state): State<AppState>,
    body: Result<Json<SearchBody>, JsonRejection>,
) -> ApiResult<Json<Value>> {
    let Json(body) = body?;
    let k = in_range("k", body.k.unwrap_or(10), MAX_K)?;
    let shared = state.clone();
    reading(&state, move |db| {
        let namespace = filter(&body.namespace);
        let model = filter(&body.embed_model_id);
        let stored;
        let query: &[f32] = match (&body.id, &body.vector) {
            (Some(id), None) => {
                stored = db.get(id).ok_or_else(|| ApiError::not_found(id))?;
                &stored.vector
            }
            (None, Some(v)) if !v.is_empty() => {
                if v.iter().any(|x| !x.is_finite()) {
                    return Err(ApiError::invalid("INVALID_VECTOR", "The query has a value too large for a 32-bit float."));
                }
                v
            }
            _ => {
                return Err(ApiError::invalid("INVALID_QUERY", "Send either an id or a non-empty vector, not both."))
            }
        };
        let started = Instant::now();
        // By id, ask for one extra: the vector itself will usually take a slot.
        let wanted = if body.id.is_some() { k + 1 } else { k };
        let hits = db.search(query, wanted, namespace, model)?;
        let took = started.elapsed().as_micros() as u64;
        shared.queries.fetch_add(1, Ordering::Relaxed);
        shared.query_micros.fetch_add(took, Ordering::Relaxed);
        let searched = db.entries().iter().filter(|e| matches(e, namespace, model)).count();
        let data: Vec<Value> = hits
            .into_iter()
            .filter(|h| body.id.as_ref() != Some(&h.id))
            .take(k)
            .map(|h| {
                json!({
                    "id": h.id,
                    "score": h.score,
                    "namespace": h.namespace,
                    "embed_model_id": h.embed_model_id,
                    "metadata": h.metadata,
                })
            })
            .collect();
        Ok(Json(json!({ "data": data, "took_micros": took, "searched": searched })))
    })
    .await
}

#[derive(Deserialize)]
struct ProjectionParams {
    namespace: Option<String>,
    embed_model_id: Option<String>,
    limit: Option<usize>,
}

async fn projection(
    State(state): State<AppState>,
    params: Result<Query<ProjectionParams>, QueryRejection>,
) -> ApiResult<Json<Value>> {
    let Query(params) = params?;
    let limit = in_range("limit", params.limit.unwrap_or(2000), MAX_PROJECTION)?;
    reading(&state, move |db| {
        let namespace = filter(&params.namespace);
        let model = filter(&params.embed_model_id);
        let matching: Vec<&Entry> = db.entries().iter().filter(|e| matches(e, namespace, model)).collect();
        let total = matching.len();
        // An even stride over storage order: stable between calls, and it doesn't favour
        // whichever namespace was loaded first.
        let picked: Vec<&Entry> = if total <= limit {
            matching
        } else {
            (0..limit).map(|i| matching[i * total / limit]).collect()
        };
        let vectors: Vec<&[f32]> = picked.iter().map(|e| e.vector.as_slice()).collect();
        let p = projection::project(&vectors);
        let points: Vec<Value> = picked
            .iter()
            .zip(&p.coords)
            .map(|(e, c)| {
                json!({
                    "id": e.id,
                    "namespace": e.namespace,
                    "embed_model_id": e.embed_model_id,
                    "x": c[0], "y": c[1], "z": c[2],
                })
            })
            .collect();
        Ok(Json(json!({
            "points": points,
            "explained": p.explained,
            "total": total,
            "dimensions": db.dimensions(),
        })))
    })
    .await
}

// ---- maintenance ----------------------------------------------------------------------------

async fn create_snapshot(State(state): State<AppState>) -> ApiResult<(StatusCode, Json<Value>)> {
    if !state.config.writable {
        return Err(ApiError::read_only());
    }
    let disk = writing(&state, |db| {
        db.snapshot()?;
        Ok(db.stats().disk)
    })
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "snapshot_bytes": disk.snapshot_bytes,
            "wal_bytes": disk.wal_bytes,
            "snapshot_modified_unix": disk.snapshot_modified_unix,
        })),
    ))
}

/// Read-only servers: load the store from disk again, to see what its writer has done.
async fn reload(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    if state.config.writable {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "ALREADY_CURRENT",
            "A writable server is always current: it holds the store itself.",
        ));
    }
    let shared = state.clone();
    let vectors = tokio::task::spawn_blocking(move || -> ApiResult<usize> {
        // Load before taking the lock, so readers keep working for the whole of it.
        let fresh = Database::open_read_only(&shared.config.data)?;
        let vectors = fresh.len();
        *shared.db.write().map_err(|_| ApiError::internal())? = fresh;
        Ok(vectors)
    })
    .await
    .map_err(|_| ApiError::internal())??;
    Ok(Json(json!({ "vectors": vectors })))
}
