//! The HTTP contract: status codes, the one error shape, and the guards.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use athenadb::core::{DbError, Entry};
use athenadb::db::Database;
use athenadb::server::{app, Config};
use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

fn config(dir: &Path, writable: bool) -> Config {
    Config { data: dir.to_path_buf(), writable, token: None, loopback_only: true }
}

fn writable(dir: &Path) -> Router {
    app(config(dir, true)).unwrap()
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn call(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder().method(method).uri(uri).header(header::CONTENT_TYPE, "application/json");
    let body = body.map_or(Body::empty(), |b| Body::from(b.to_string()));
    send(app, req.body(body).unwrap()).await
}

async fn seed(app: &Router) {
    let (status, body) = call(
        app,
        Method::POST,
        "/api/vectors",
        Some(json!({ "vectors": [
            { "id": "cat", "vector": [0.9, 0.1, 0.0], "namespace": "pets", "embed_model_id": "m1", "metadata": { "kind": "animal" } },
            { "id": "kitten", "vector": [0.85, 0.2, 0.0], "namespace": "pets", "embed_model_id": "m1" },
            { "id": "car", "vector": [0.0, 0.1, 0.95], "namespace": "vehicles", "embed_model_id": "m2" },
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body, json!({ "added": 3, "vectors": 3 }));
}

fn assert_error(body: &Value, code: &str) {
    assert_eq!(body["error"]["code"], code, "{body}");
    assert!(body["error"]["message"].as_str().is_some_and(|m| !m.is_empty()), "{body}");
}

#[tokio::test]
async fn stats_describe_an_empty_store() {
    let dir = tempfile::tempdir().unwrap();
    let (status, body) = call(&writable(dir.path()), Method::GET, "/api/stats", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["vectors"], 0);
    assert_eq!(body["dimensions"], Value::Null);
    assert_eq!(body["read_only"], false);
    assert_eq!(body["groups"], json!([]));
    assert_eq!(body["durability"]["snapshot_modified_unix"], Value::Null);
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn stats_group_by_namespace_and_model() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    seed(&app).await;
    let (_, body) = call(&app, Method::GET, "/api/stats", None).await;
    assert_eq!(body["vectors"], 3);
    assert_eq!(body["dimensions"], 3);
    assert_eq!(body["groups"][0], json!({ "namespace": "pets", "embed_model_id": "m1", "vectors": 2 }));
    assert_eq!(body["groups"][1], json!({ "namespace": "vehicles", "embed_model_id": "m2", "vectors": 1 }));
    assert_eq!(body["durability"]["ops_since_snapshot"], 3);
}

#[tokio::test]
async fn search_by_vector_ranks_by_cosine() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    seed(&app).await;
    let (status, body) =
        call(&app, Method::POST, "/api/search", Some(json!({ "vector": [1.0, 0.0, 0.0], "k": 2 }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"][0]["id"], "cat");
    assert_eq!(body["data"][1]["id"], "kitten");
    assert_eq!(body["data"].as_array().unwrap().len(), 2);
    assert_eq!(body["searched"], 3);
    assert_eq!(body["data"][0]["metadata"], json!({ "kind": "animal" }));
    assert!(body["took_micros"].is_u64());
}

#[tokio::test]
async fn search_by_id_leaves_the_vector_itself_out() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    seed(&app).await;
    let (_, body) = call(&app, Method::POST, "/api/search", Some(json!({ "id": "cat", "k": 1 }))).await;
    assert_eq!(body["data"].as_array().unwrap().len(), 1);
    assert_eq!(body["data"][0]["id"], "kitten");
}

#[tokio::test]
async fn search_respects_filters() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    seed(&app).await;
    let (_, body) = call(
        &app,
        Method::POST,
        "/api/search",
        Some(json!({ "vector": [1.0, 0.0, 0.0], "namespace": "vehicles" })),
    )
    .await;
    assert_eq!(body["data"][0]["id"], "car");
    assert_eq!(body["searched"], 1);
}

#[tokio::test]
async fn bad_searches_use_the_one_error_shape() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    seed(&app).await;
    let cases = [
        (json!({ "vector": [1.0], "k": 0 }), StatusCode::UNPROCESSABLE_ENTITY, "OUT_OF_RANGE"),
        (json!({ "vector": [1.0, 0.0, 0.0], "k": 1001 }), StatusCode::UNPROCESSABLE_ENTITY, "OUT_OF_RANGE"),
        (json!({}), StatusCode::UNPROCESSABLE_ENTITY, "INVALID_QUERY"),
        (json!({ "vector": [] }), StatusCode::UNPROCESSABLE_ENTITY, "INVALID_QUERY"),
        (json!({ "id": "cat", "vector": [1.0, 0.0, 0.0] }), StatusCode::UNPROCESSABLE_ENTITY, "INVALID_QUERY"),
        (json!({ "vector": [1.0, 0.0] }), StatusCode::UNPROCESSABLE_ENTITY, "DIMENSION_MISMATCH"),
        (json!({ "vector": [1e40, 0.0, 0.0] }), StatusCode::UNPROCESSABLE_ENTITY, "INVALID_VECTOR"),
        (json!({ "id": "missing" }), StatusCode::NOT_FOUND, "NOT_FOUND"),
        (json!({ "vector": "nope" }), StatusCode::UNPROCESSABLE_ENTITY, "INVALID_JSON"),
    ];
    for (body, status, code) in cases {
        let (got, res) = call(&app, Method::POST, "/api/search", Some(body.clone())).await;
        assert_eq!(got, status, "{body} -> {res}");
        assert_error(&res, code);
    }
    let bad_json = Request::builder()
        .method(Method::POST)
        .uri("/api/search")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let (status, res) = send(&app, bad_json).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&res, "INVALID_JSON");
}

#[tokio::test]
async fn listing_filters_and_paginates() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    seed(&app).await;

    let (_, all) = call(&app, Method::GET, "/api/vectors", None).await;
    assert_eq!(all["pagination"], json!({ "limit": 50, "offset": 0, "total": 3 }));
    assert!(all["data"][0].get("vector").is_none(), "lists leave the vectors out");

    let (_, page) = call(&app, Method::GET, "/api/vectors?limit=1&offset=1", None).await;
    assert_eq!(page["data"].as_array().unwrap().len(), 1);
    assert_eq!(page["data"][0]["id"], "kitten");
    assert_eq!(page["pagination"]["total"], 3);

    let (_, ns) = call(&app, Method::GET, "/api/vectors?namespace=vehicles", None).await;
    assert_eq!(ns["pagination"]["total"], 1);

    let (_, model) = call(&app, Method::GET, "/api/vectors?embed_model_id=m1", None).await;
    assert_eq!(model["pagination"]["total"], 2);

    let (_, q) = call(&app, Method::GET, "/api/vectors?q=KIT", None).await;
    assert_eq!(q["data"][0]["id"], "kitten");
    assert_eq!(q["pagination"]["total"], 1);

    // An empty filter from a form means no filter.
    let (_, blank) = call(&app, Method::GET, "/api/vectors?namespace=&q=", None).await;
    assert_eq!(blank["pagination"]["total"], 3);

    let (status, res) = call(&app, Method::GET, "/api/vectors?limit=0", None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_error(&res, "OUT_OF_RANGE");
    let (status, res) = call(&app, Method::GET, "/api/vectors?limit=abc", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_error(&res, "INVALID_QUERY");
}

#[tokio::test]
async fn get_and_delete_one_vector() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    seed(&app).await;

    let (status, body) = call(&app, Method::GET, "/api/vectors/cat", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["vector"].as_array().unwrap().len(), 3);
    assert_eq!(body["metadata"], json!({ "kind": "animal" }));

    let (status, res) = call(&app, Method::GET, "/api/vectors/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&res, "NOT_FOUND");

    let (status, body) = call(&app, Method::DELETE, "/api/vectors/cat", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "deleted": true }));
    // Safe to retry.
    let (status, body) = call(&app, Method::DELETE, "/api/vectors/cat", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "deleted": false }));
}

#[tokio::test]
async fn ids_with_awkward_characters_work_when_encoded() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    let (status, _) = call(
        &app,
        Method::POST,
        "/api/vectors",
        Some(json!({ "vectors": [{ "id": "docs/a b?.md", "vector": [1.0, 0.0] }] })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = call(&app, Method::GET, "/api/vectors/docs%2Fa%20b%3F.md", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], "docs/a b?.md");
    assert_eq!(body["namespace"], "default");
}

#[tokio::test]
async fn a_bad_batch_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    seed(&app).await;
    let cases = [
        (json!({ "vectors": [] }), "EMPTY_BATCH"),
        (json!({ "vectors": [{ "id": "", "vector": [1.0, 0.0, 0.0] }] }), "INVALID_ID"),
        (json!({ "vectors": [{ "id": "a", "vector": [] }] }), "INVALID_VECTOR"),
        (json!({ "vectors": [{ "id": "a", "vector": [1e300, 0.0, 0.0] }] }), "INVALID_VECTOR"),
        (
            json!({ "vectors": [{ "id": "ok", "vector": [1.0, 0.0, 0.0] }, { "id": "bad", "vector": [1.0, 0.0] }] }),
            "DIMENSION_MISMATCH",
        ),
    ];
    for (body, code) in cases {
        let (status, res) = call(&app, Method::POST, "/api/vectors", Some(body.clone())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body} -> {res}");
        assert_error(&res, code);
    }
    let (_, stats) = call(&app, Method::GET, "/api/stats", None).await;
    assert_eq!(stats["vectors"], 3);
}

#[tokio::test]
async fn snapshots_empty_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    seed(&app).await;
    let (status, body) = call(&app, Method::POST, "/api/snapshots", None).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["wal_bytes"], 0);
    assert!(body["snapshot_bytes"].as_u64().unwrap() > 0);
    assert!(body["snapshot_modified_unix"].as_u64().unwrap() > 1_700_000_000, "seconds since the epoch");
    let (_, stats) = call(&app, Method::GET, "/api/stats", None).await;
    assert_eq!(stats["durability"]["ops_since_snapshot"], 0);
}

#[tokio::test]
async fn projection_returns_one_point_per_sampled_vector() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    seed(&app).await;
    let (status, body) = call(&app, Method::GET, "/api/projection", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["points"].as_array().unwrap().len(), 3);
    assert_eq!(body["total"], 3);
    assert_eq!(body["dimensions"], 3);
    assert!(body["points"][0]["x"].is_number());
    assert_eq!(body["explained"].as_array().unwrap().len(), 3);

    let (_, sampled) = call(&app, Method::GET, "/api/projection?limit=2", None).await;
    assert_eq!(sampled["points"].as_array().unwrap().len(), 2);
    assert_eq!(sampled["total"], 3);

    let (_, filtered) = call(&app, Method::GET, "/api/projection?namespace=vehicles", None).await;
    assert_eq!(filtered["points"][0]["id"], "car");
}

#[tokio::test]
async fn an_empty_store_has_an_empty_projection() {
    let dir = tempfile::tempdir().unwrap();
    let (status, body) = call(&writable(dir.path()), Method::GET, "/api/projection", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["points"], json!([]));
    assert_eq!(body["dimensions"], Value::Null);
}

fn entry(id: &str, v: Vec<f32>) -> Entry {
    Entry {
        id: id.into(),
        vector: v,
        metadata: Value::Null,
        namespace: "default".into(),
        embed_model_id: String::new(),
        inv_norm: 0.0,
    }
}

#[tokio::test]
async fn read_only_refuses_writes_and_reloads_to_catch_up() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = Database::open(dir.path()).unwrap();
    writer.add(entry("a", vec![1.0, 0.0])).unwrap();

    let app = app(config(dir.path(), false)).unwrap();
    let (_, stats) = call(&app, Method::GET, "/api/stats", None).await;
    assert_eq!(stats["read_only"], true);
    assert_eq!(stats["vectors"], 1);
    assert_eq!(stats["durability"]["disk_changed"], false);

    let add = json!({ "vectors": [{ "id": "b", "vector": [0.0, 1.0] }] });
    for (method, uri, body) in [
        (Method::POST, "/api/vectors", Some(add)),
        (Method::DELETE, "/api/vectors/a", None),
        (Method::POST, "/api/snapshots", None),
    ] {
        let (status, res) = call(&app, method, uri, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
        assert_error(&res, "READ_ONLY");
    }

    // Reads still work.
    let (status, _) = call(&app, Method::POST, "/api/search", Some(json!({ "id": "a" }))).await;
    assert_eq!(status, StatusCode::OK);

    // The other process writes; the dashboard notices, then catches up on request.
    writer.add(entry("c", vec![0.0, 1.0])).unwrap();
    let (_, stats) = call(&app, Method::GET, "/api/stats", None).await;
    assert_eq!(stats["durability"]["disk_changed"], true);
    assert_eq!(stats["vectors"], 1);
    let (status, body) = call(&app, Method::POST, "/api/reload", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["vectors"], 2);
    let (_, stats) = call(&app, Method::GET, "/api/stats", None).await;
    assert_eq!(stats["durability"]["disk_changed"], false);
}

#[tokio::test]
async fn a_writable_server_has_nothing_to_reload() {
    let dir = tempfile::tempdir().unwrap();
    let (status, res) = call(&writable(dir.path()), Method::POST, "/api/reload", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_error(&res, "ALREADY_CURRENT");
}

#[tokio::test]
async fn a_read_only_server_needs_an_existing_store() {
    let dir = tempfile::tempdir().unwrap();
    assert!(app(config(&dir.path().join("nope"), false)).is_err());
    assert!(!dir.path().join("nope").exists());
}

#[tokio::test]
async fn rebinding_is_refused_by_the_host_check() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    for (host, allowed) in [
        ("localhost:8001", true),
        ("127.0.0.1:8001", true),
        ("[::1]:8001", true),
        ("localhost", true),
        ("evil.example:8001", false),
        ("localhost.evil.example", false),
        ("127.0.0.1.evil.example:8001", false),
    ] {
        let req = Request::builder().uri("/api/stats").header(header::HOST, host).body(Body::empty()).unwrap();
        let (status, res) = send(&app, req).await;
        if allowed {
            assert_eq!(status, StatusCode::OK, "{host}");
        } else {
            assert_eq!(status, StatusCode::FORBIDDEN, "{host}");
            assert_error(&res, "FORBIDDEN_HOST");
        }
    }
}

#[tokio::test]
async fn a_cross_site_form_post_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    for content_type in ["text/plain", "application/x-www-form-urlencoded", "multipart/form-data"] {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/snapshots")
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::empty())
            .unwrap();
        let (status, res) = send(&app, req).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{content_type}");
        assert_error(&res, "JSON_REQUIRED");
    }
    assert!(!dir.path().join("snapshot.bin").exists());
}

#[tokio::test]
async fn a_token_guards_the_api_but_not_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(Config {
        data: dir.path().to_path_buf(),
        writable: true,
        token: Some("s3cret".into()),
        loopback_only: false,
    })
    .unwrap();
    let get = |uri: &'static str, auth: Option<&'static str>| {
        let mut r = Request::builder().uri(uri);
        if let Some(a) = auth {
            r = r.header(header::AUTHORIZATION, a);
        }
        r.body(Body::empty()).unwrap()
    };
    let (status, res) = send(&app, get("/api/stats", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_error(&res, "UNAUTHORIZED");
    assert_eq!(send(&app, get("/api/stats", Some("Bearer wrong!"))).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(send(&app, get("/api/stats", Some("Bearer s3cret"))).await.0, StatusCode::OK);
    assert_eq!(send(&app, get("/api/health", None)).await.0, StatusCode::OK);
    assert_eq!(send(&app, get("/", None)).await.0, StatusCode::OK);
}

#[tokio::test]
async fn the_dashboard_is_served_with_a_strict_policy() {
    let dir = tempfile::tempdir().unwrap();
    let app = writable(dir.path());
    let res = app.clone().oneshot(Request::builder().uri("/").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/html"));
    let csp = res.headers()[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    assert!(csp.contains("default-src 'none'") && csp.contains("script-src 'self'"), "{csp}");
    assert_eq!(res.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");

    let js = app.clone().oneshot(Request::builder().uri("/assets/app.js").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(js.status(), StatusCode::OK);
    assert!(js.headers()[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/javascript"));

    let page = app.clone().oneshot(Request::builder().uri("/assets/pages/search.js").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(page.status(), StatusCode::OK);

    let missing = app.clone().oneshot(Request::builder().uri("/assets/nope.js").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let (status, res) = call(&app, Method::GET, "/api/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error(&res, "NOT_FOUND");
}

#[test]
fn a_writable_server_is_refused_a_store_someone_else_is_writing() {
    let dir = tempfile::tempdir().unwrap();
    let _writer = Database::open(dir.path()).unwrap();

    assert!(matches!(app(config(dir.path(), true)), Err(DbError::Locked { .. })));
    assert!(app(config(dir.path(), false)).is_ok(), "read-only must still open");
}

#[test]
fn the_command_explains_a_locked_store_and_how_to_get_past_it() {
    let dir = tempfile::tempdir().unwrap();
    let _writer = Database::open(dir.path()).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_athenadb"))
        .arg("--data")
        .arg(dir.path())
        .args(["--writable", "--port", "0"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("it started serving a store another writer holds");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&dir.path().display().to_string()), "names the store: {stderr}");
    assert!(stderr.contains("without --writable"), "says how to get past it: {stderr}");
}
