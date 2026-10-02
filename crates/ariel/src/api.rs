use crate::{
    App,
    config::{self, id, now},
    runtime, store, workspace,
};
use anyhow::{Context, Result};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{
        Html, IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{convert::Infallible, sync::Arc, time::Duration};
macro_rules! ensure {
    ($condition:expr,$message:expr) => {
        if !$condition {
            return Err(anyhow::anyhow!($message).into());
        }
    };
}
macro_rules! bail {
    ($message:expr) => {
        return Err(anyhow::anyhow!($message).into())
    };
}

#[derive(Clone)]
pub struct Principal {
    pub id: String,
    pub owner: bool,
}
pub struct ApiError(StatusCode, String);
impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        let message = e.to_string();
        let safe =
            if message.len() < 100 && message.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                message
            } else {
                "operation_failed".into()
            };
        let code = if safe == "not_found" {
            StatusCode::NOT_FOUND
        } else if safe == "forbidden" {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::CONFLICT
        };
        Self(code, safe)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error":{"code":self.1}}))).into_response()
    }
}
type Reply = std::result::Result<Json<Value>, ApiError>;
fn owner(p: &Principal) -> Result<()> {
    ensure!(p.owner, "forbidden");
    Ok(())
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key].as_str().context("required_field_missing")
}
fn own(app: &App, p: &Principal, kind: &str, id: &str) -> Result<Value> {
    let value = app.store.get(kind, id)?.context("not_found")?;
    ensure!(p.owner || value["client_id"] == p.id, "not_found");
    Ok(value)
}
fn workspace_public(mut v: Value, owner: bool) -> Value {
    if !owner && let Some(grants) = v["grants"].as_array_mut() {
        for g in grants {
            g.as_object_mut().unwrap().retain(|k, _| {
                [
                    "id",
                    "session_id",
                    "access",
                    "target",
                    "expires_at",
                    "state",
                ]
                .contains(&k.as_str())
            });
        }
    }
    v
}
pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../../../web/admin/index.html")) }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [("content-type", "text/javascript; charset=utf-8")],
                    include_str!("../../../web/admin/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [("content-type", "text/css; charset=utf-8")],
                    include_str!("../../../web/style.css"),
                )
            }),
        )
        .route("/health", get(health))
        .route("/v1/admin/catalog", get(catalog))
        .route("/v1/admin/clients", get(clients))
        .route("/v1/admin/providers/{id}", put(provider))
        .route("/v1/admin/models/{id}", put(model))
        .route("/v1/admin/refresh", post(refresh))
        .route("/v1/admin/import-pi", post(import))
        .route("/v1/admin/shutdown", post(shutdown))
        .route("/v1/agents", get(agents))
        .route("/v1/agents/{id}/models", get(models))
        .route("/v1/workspaces", get(workspaces).post(prepare))
        .route("/v1/workspaces/{id}/files", get(files))
        .route("/v1/workspaces/{id}/file", get(file))
        .route("/v1/workspaces/{id}/grants/{grant}/revoke", post(revoke))
        .route("/v1/sessions", get(sessions).post(session))
        .route("/v1/jobs", get(jobs).post(submit))
        .route("/v1/jobs/{id}", get(job))
        .route("/v1/jobs/{id}/events", get(events))
        .route("/v1/jobs/{id}/cancel", post(cancel))
        .route("/v1/questions", get(questions))
        .route("/v1/questions/{id}/answer", post(answer))
        .route("/v1/access-requests/{id}/decision", post(decision))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn_with_state(app.clone(), auth))
        .with_state(app)
}
pub fn client_router(app: Arc<App>) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../../../examples/pi-client/index.html")) }),
        )
        .route(
            "/client.js",
            get(|| async {
                (
                    [("content-type", "text/javascript; charset=utf-8")],
                    include_str!("../../../examples/pi-client/client.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [("content-type", "text/css; charset=utf-8")],
                    include_str!("../../../web/style.css"),
                )
            }),
        )
        .layer(middleware::from_fn_with_state(app, client_security))
}
async fn client_security(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let port = app.config.test_bind.rsplit(':').next().unwrap();
    let host = request
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut response = next.run(request).await;
    let api_port = app.config.bind.rsplit(':').next().unwrap();
    let csp = format!(
        "default-src 'self'; script-src 'self'; style-src 'self'; connect-src http://127.0.0.1:{api_port} http://localhost:{api_port}; object-src 'none'; frame-ancestors 'none'; base-uri 'none'"
    );
    response.headers_mut().insert(
        "content-security-policy",
        HeaderValue::from_str(&csp).unwrap(),
    );
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}
fn equal(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0, |acc, (a, b)| acc | (a ^ b))
        == 0
}
async fn auth(State(app): State<Arc<App>>, mut request: Request, next: Next) -> Response {
    let port = app.config.bind.rsplit(':').next().unwrap_or("8787");
    let host = request
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return ApiError(StatusCode::FORBIDDEN, "invalid_host".into()).into_response();
    }
    let origin = request
        .headers()
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let test_port = app.config.test_bind.rsplit(':').next().unwrap_or("8788");
    let allowed = [
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
        format!("http://127.0.0.1:{test_port}"),
        format!("http://localhost:{test_port}"),
    ];
    if origin.as_ref().is_some_and(|o| !allowed.contains(o)) {
        return ApiError(StatusCode::FORBIDDEN, "invalid_origin".into()).into_response();
    }
    let mut response = if request.method() == axum::http::Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else {
        if request.uri().path().starts_with("/v1/") {
            let token = request
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer "))
                .unwrap_or("");
            let principal = if equal(token, &app.config.owner_token) {
                Some(Principal {
                    id: "owner".into(),
                    owner: true,
                })
            } else {
                app.config
                    .clients
                    .iter()
                    .find(|c| equal(&c.token, token))
                    .map(|c| Principal {
                        id: c.id.clone(),
                        owner: false,
                    })
            };
            let Some(p) = principal else {
                return ApiError(StatusCode::UNAUTHORIZED, "authentication_required".into())
                    .into_response();
            };
            request.extensions_mut().insert(p);
        }
        next.run(request).await
    };
    if let Some(o) = origin {
        response.headers_mut().insert(
            "access-control-allow-origin",
            HeaderValue::from_str(&o).unwrap(),
        );
        response
            .headers_mut()
            .insert("vary", HeaderValue::from_static("Origin"));
        response.headers_mut().insert(
            "access-control-allow-headers",
            HeaderValue::from_static("Authorization, Content-Type, Last-Event-ID"),
        );
        response.headers_mut().insert(
            "access-control-allow-methods",
            HeaderValue::from_static("GET, POST, PUT, OPTIONS"),
        );
    }
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response.headers_mut().insert("content-security-policy",HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'"));
    response
}
async fn health(State(app): State<Arc<App>>) -> Json<Value> {
    Json(
        json!({"name":"Ariel","version":env!("CARGO_PKG_VERSION"),"pi_version":config::PI_VERSION,"runtime_ready":app.ready,"test_client_url":format!("http://{}/",app.config.test_bind),"isolation":"docker-host-filesystem","network":"docker-bridge (egress allowlist unsupported)","limits":{"memory_bytes":536870912,"cpus":1,"pids":128,"workspace_bytes":134217728,"workspace_limit_enforcement":"observed-stop; hard disk quota unsupported"}}),
    )
}
async fn catalog(State(app): State<Arc<App>>, Extension(p): Extension<Principal>) -> Reply {
    owner(&p)?;
    Ok(Json(
        json!({"catalog":app.projection(true)?,"providers":app.store.list("provider",None)?,"runtime_ready":app.ready,"import":config::read_json(&app.root.join("profile/import-summary.json")).unwrap_or(json!({}))}),
    ))
}
async fn clients(State(app): State<Arc<App>>, Extension(p): Extension<Principal>) -> Reply {
    owner(&p)?;
    Ok(Json(json!({"clients":app.config.clients})))
}
async fn provider(
    State(app): State<Arc<App>>,
    Extension(p): Extension<Principal>,
    Path(pid): Path<String>,
    Json(body): Json<Value>,
) -> Reply {
    owner(&p)?;
    let _lock = app.admission.lock().await;
    ensure!(
        app.store.get("provider", &pid)?.is_some(),
        "unknown_provider"
    );
    let enabled = body["enabled"].as_bool().context("enabled_required")?;
    if let Some(key) = body["api_key"].as_str() {
        ensure!(
            !key.is_empty() && key.len() <= 8192 && !key.starts_with('!'),
            "invalid_api_key"
        );
        let file = app.root.join("profile/auth.json");
        let mut auth = config::read_json(&file)?;
        auth[&pid] = json!({"type":"api_key","key":key});
        config::write_json(&file, &auth)?;
        let fresh = crate::read_catalog(&app.root, &app.config).await?;
        app.replace_catalog(fresh)?;
    }
    let updated = app.store.change("provider", &pid, |v| {
        v["enabled"] = json!(enabled);
        if body["api_key"].is_string() {
            v["revision"] = json!(v["revision"].as_u64().unwrap_or(0) + 1);
        }
        Ok(())
    })?;
    if !enabled || body["api_key"].is_string() {
        stop_matching(&app, Some(&pid), None, None, "revoke").await?;
    }
    Ok(Json(updated))
}
async fn model(
    State(app): State<Arc<App>>,
    Extension(p): Extension<Principal>,
    Path(mid): Path<String>,
    Json(body): Json<Value>,
) -> Reply {
    owner(&p)?;
    let _lock = app.admission.lock().await;
    let enabled = body["enabled"].as_bool().context("enabled_required")?;
    let updated = app.store.change("model", &mid, |v| {
        v["enabled"] = json!(enabled);
        Ok(())
    })?;
    if !enabled {
        stop_matching(
            &app,
            updated["provider_id"].as_str(),
            updated["model_id"].as_str(),
            None,
            "revoke",
        )
        .await?;
    }
    Ok(Json(updated))
}
async fn refresh(State(app): State<Arc<App>>, Extension(p): Extension<Principal>) -> Reply {
    owner(&p)?;
    let _lock = app.admission.lock().await;
    let fresh = crate::read_catalog(&app.root, &app.config).await?;
    app.replace_catalog(fresh)?;
    Ok(Json(app.projection(true)?))
}
async fn import(State(app): State<Arc<App>>, Extension(p): Extension<Principal>) -> Reply {
    owner(&p)?;
    let _lock = app.admission.lock().await;
    let summary = config::import_pi(&app.root)?;
    for provider in app.store.list("provider", None)? {
        app.store.change("provider", text(&provider, "id")?, |v| {
            v["revision"] = json!(v["revision"].as_u64().unwrap_or(0) + 1);
            Ok(())
        })?;
    }
    let fresh = crate::read_catalog(&app.root, &app.config).await?;
    app.replace_catalog(fresh)?;
    stop_matching(&app, None, None, None, "revoke").await?;
    Ok(Json(summary))
}
async fn shutdown(State(app): State<Arc<App>>, Extension(p): Extension<Principal>) -> Reply {
    owner(&p)?;
    app.stop_notify.notify_one();
    Ok(Json(json!({"state":"stopping"})))
}
async fn agents(State(app): State<Arc<App>>) -> Json<Value> {
    Json(
        json!({"agents":[{"id":"pi","version":config::PI_VERSION,"transport":"rpc-jsonl","ready":app.ready,"capabilities":["tools","stream","sessions","cancel","host_filesystem_isolation","path_grants","questions"],"unsupported":["hard_disk_quota","egress_allowlist","protected_internal_paths","privacy_scanner"]}]}),
    )
}
async fn models(State(app): State<Arc<App>>, Path(agent): Path<String>) -> Reply {
    ensure!(agent == "pi", "unknown_agent");
    Ok(Json(app.projection(false)?))
}
async fn workspaces(State(app): State<Arc<App>>, Extension(p): Extension<Principal>) -> Reply {
    Ok(Json(
        json!({"workspaces":app.store.list("workspace",if p.owner{None}else{Some(&p.id)})?.into_iter().map(|v|workspace_public(v,p.owner)).collect::<Vec<_>>()}),
    ))
}
async fn prepare(
    State(app): State<Arc<App>>,
    Extension(p): Extension<Principal>,
    Json(body): Json<Value>,
) -> Reply {
    if !p.owner {
        ensure!(
            body["mode"].as_str().unwrap_or("folder") == "folder" && body["source_path"].is_null(),
            "forbidden"
        );
    }
    let client = if p.owner {
        body["client_id"].as_str().unwrap_or("test-client")
    } else {
        &p.id
    };
    let value = workspace::prepare(&app, client, &body).await?;
    Ok(Json(workspace_public(value, p.owner)))
}
async fn sessions(State(app): State<Arc<App>>, Extension(p): Extension<Principal>) -> Reply {
    Ok(Json(
        json!({"sessions":app.store.list("session",if p.owner{None}else{Some(&p.id)})?}),
    ))
}
async fn session(
    State(app): State<Arc<App>>,
    Extension(p): Extension<Principal>,
    Json(body): Json<Value>,
) -> Reply {
    let _lock = app.admission.lock().await;
    ensure!(
        body["agent_id"].as_str().unwrap_or("pi") == "pi",
        "unknown_agent"
    );
    if let Some(required) = body["required_capabilities"].as_array() {
        for cap in required {
            ensure!(
                [
                    "host_filesystem_isolation",
                    "cancel",
                    "stream",
                    "tools",
                    "sessions",
                    "path_grants",
                    "questions"
                ]
                .contains(&cap.as_str().unwrap_or("")),
                "unsupported_required_isolation"
            );
        }
    }
    let provider = text(&body, "provider_id")?;
    let model = text(&body, "model_id")?;
    app.admitted(provider, model)?;
    let workspace = own(&app, &p, "workspace", text(&body, "workspace_id")?)?;
    let sid = id();
    let revision = app.store.get("provider", provider)?.unwrap()["revision"].clone();
    let value = json!({"id":sid,"client_id":workspace["client_id"],"agent_id":"pi","provider_id":provider,"model_id":model,"workspace_id":workspace["id"],"profile_revision":revision,"blocked":false,"created_at":now()});
    workspace::session_profile(&app, &value)?;
    app.store
        .put("session", &sid, text(&workspace, "client_id")?, &value)?;
    Ok(Json(value))
}
async fn jobs(State(app): State<Arc<App>>, Extension(p): Extension<Principal>) -> Reply {
    Ok(Json(
        json!({"jobs":app.store.list("job",if p.owner{None}else{Some(&p.id)})?.into_iter().rev().take(100).collect::<Vec<_>>()}),
    ))
}
async fn submit(
    State(app): State<Arc<App>>,
    Extension(p): Extension<Principal>,
    Json(body): Json<Value>,
) -> Reply {
    let _lock = app.admission.lock().await;
    let session = own(&app, &p, "session", text(&body, "session_id")?)?;
    let sid = text(&session, "id")?;
    let client = text(&session, "client_id")?;
    let input = text(&body, "input")?;
    let key = text(&body, "idempotency_key")?;
    ensure!(
        !input.trim().is_empty() && input.len() <= 32000 && !key.is_empty() && key.len() <= 128,
        "invalid_job_input"
    );
    let timeout = body["timeout_seconds"].as_u64().unwrap_or(300);
    ensure!((30..=600).contains(&timeout), "invalid_deadline");
    let fingerprint = config::digest(&format!("{sid}\0{input}\0{timeout}"));
    // Check a retry before rejecting another active job in the same workspace.
    let active = app.store.list("job", Some(client))?;
    let retry = active.iter().any(|j| j["idempotency_key"] == key);
    if retry {
        let (job, _) = app
            .store
            .submit(client, sid, input, key, &fingerprint, now() + timeout)?;
        return Ok(Json(job));
    }
    ensure!(session["blocked"] != true, "session_blocked");
    let provider = text(&session, "provider_id")?;
    let model = text(&session, "model_id")?;
    app.admitted(provider, model)?;
    ensure!(
        app.store.get("provider", provider)?.unwrap()["revision"] == session["profile_revision"],
        "profile_revision_changed"
    );
    if !retry {
        for j in &active {
            if !store::terminal(j["state"].as_str().unwrap_or("")) {
                let s = app
                    .store
                    .get("session", text(j, "session_id")?)?
                    .context("session_missing")?;
                ensure!(
                    s["workspace_id"] != session["workspace_id"],
                    "workspace_busy"
                );
            }
        }
    }
    let (job, new) = app
        .store
        .submit(client, sid, input, key, &fingerprint, now() + timeout)?;
    if new {
        let (tx, rx) = tokio::sync::watch::channel(String::new());
        app.controls
            .lock()
            .await
            .insert(text(&job, "id")?.into(), tx);
        tokio::spawn(runtime::run(app.clone(), job.clone(), rx));
    }
    Ok(Json(job))
}
async fn job(
    State(app): State<Arc<App>>,
    Extension(p): Extension<Principal>,
    Path(jid): Path<String>,
) -> Reply {
    Ok(Json(own(&app, &p, "job", &jid)?))
}
async fn cancel(
    State(app): State<Arc<App>>,
    Extension(p): Extension<Principal>,
    Path(jid): Path<String>,
) -> Reply {
    let _lock = app.admission.lock().await;
    let j = own(&app, &p, "job", &jid)?;
    if !store::terminal(text(&j, "state")?) {
        app.store.change("job", &jid, |v| {
            v["state"] = json!("cancelling");
            Ok(())
        })?;
        if let Some(tx) = app.controls.lock().await.get(&jid) {
            let _ = tx.send("cancel".into());
        }
    }
    Ok(Json(app.store.get("job", &jid)?.unwrap()))
}
#[derive(Deserialize)]
struct Cursor {
    #[serde(default)]
    after: i64,
}
async fn events(
    State(app): State<Arc<App>>,
    Extension(p): Extension<Principal>,
    Path(jid): Path<String>,
    Query(cursor): Query<Cursor>,
) -> std::result::Result<Response, ApiError> {
    own(&app, &p, "job", &jid)?;
    ensure!(cursor.after >= 0, "invalid_cursor");
    let stream = async_stream::stream! {
        let mut after=cursor.after;
        loop {
            let mut batch=match app.store.events(&jid,after){Ok(v)=>v,Err(_)=>break};
            if batch.is_empty(){
                let terminal=app.store.get("job",&jid).ok().flatten().is_some_and(|j|store::terminal(j["state"].as_str().unwrap_or("")));
                if terminal { batch=app.store.events(&jid,after).unwrap_or_default();if batch.is_empty(){break;} }
            }
            if batch.is_empty(){tokio::time::sleep(Duration::from_millis(250)).await;continue;}
            for event in batch{
                after=event["cursor"].as_i64().unwrap();
                yield Ok::<Event,Infallible>(Event::default().id(after.to_string()).data(event.to_string()));
            }
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(10)))
        .into_response())
}

#[derive(Deserialize)]
struct FileQuery {
    path: String,
}
async fn files(
    State(app): State<Arc<App>>,
    Extension(p): Extension<Principal>,
    Path(wid): Path<String>,
) -> Reply {
    let w = own(&app, &p, "workspace", &wid)?;
    Ok(Json(json!({"files":workspace::files(&app,&w)?})))
}
async fn file(
    State(app): State<Arc<App>>,
    Extension(p): Extension<Principal>,
    Path(wid): Path<String>,
    Query(q): Query<FileQuery>,
) -> Reply {
    let w = own(&app, &p, "workspace", &wid)?;
    Ok(Json(workspace::file(&app, &w, &q.path)?))
}
async fn questions(State(app): State<Arc<App>>, Extension(p): Extension<Principal>) -> Reply {
    let q = app
        .store
        .list("question", if p.owner { None } else { Some(&p.id) })?
        .into_iter()
        .filter(|v| v["state"] == "pending")
        .map(|mut v| {
            if !p.owner && v["kind"] == "path" {
                v.as_object_mut().unwrap().retain(|k, _| {
                    ["id", "job_id", "kind", "state", "expires_at"].contains(&k.as_str())
                });
            }
            v
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({"questions":q})))
}
async fn answer(
    State(app): State<Arc<App>>,
    Extension(p): Extension<Principal>,
    Path(qid): Path<String>,
    Json(body): Json<Value>,
) -> Reply {
    let _lock = app.admission.lock().await;
    let q = own(&app, &p, "question", &qid)?;
    ensure!(q["kind"] == "question", "forbidden");
    validate_question(&app, &q, &body)?;
    let runtime = app
        .runtimes
        .lock()
        .await
        .get(text(&q, "session_id")?)
        .cloned()
        .context("runtime_unavailable")?;
    let mut response = json!({"type":"extension_ui_response","id":q["rpc_id"]});
    if q["method"] == "confirm" {
        response["confirmed"] = json!(body["confirmed"].as_bool().context("answer_required")?);
    } else {
        let value = text(&body, "value")?;
        ensure!(value.len() <= 8192, "answer_limit");
        if q["method"] == "select" {
            ensure!(
                q["options"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|s| s == value)),
                "invalid_option"
            );
        }
        response["value"] = json!(value);
    }
    app.store.settle_question(&qid, "answered")?;
    runtime.send(response).await?;
    Ok(Json(json!({"state":"answered"})))
}
fn validate_question(app: &App, q: &Value, body: &Value) -> Result<()> {
    ensure!(
        q["state"] == "pending" && q["expires_at"].as_u64().unwrap_or(0) > now(),
        "stale_question"
    );
    ensure!(
        q["request_digest"] == body["request_digest"],
        "request_digest_changed"
    );
    let j = app
        .store
        .get("job", text(q, "job_id")?)?
        .context("job_missing")?;
    ensure!(
        j["state"] == "waiting_input" && j["attempt_id"] == q["attempt_id"],
        "stale_attempt"
    );
    let w = app
        .store
        .get("workspace", text(q, "workspace_id")?)?
        .context("workspace_missing")?;
    ensure!(
        w["generation"] == q["workspace_generation"],
        "workspace_generation_changed"
    );
    Ok(())
}
async fn decision(
    State(app): State<Arc<App>>,
    Extension(p): Extension<Principal>,
    Path(qid): Path<String>,
    Json(body): Json<Value>,
) -> Reply {
    owner(&p)?;
    let _lock = app.admission.lock().await;
    let q = own(&app, &p, "question", &qid)?;
    ensure!(q["kind"] == "path", "invalid_access_request");
    validate_question(&app, &q, &body)?;
    let allowed = match text(&body, "decision")? {
        "allowed" => true,
        "denied" => false,
        _ => bail!("invalid_decision"),
    };
    let runtime = app
        .runtimes
        .lock()
        .await
        .get(text(&q, "session_id")?)
        .cloned()
        .context("runtime_unavailable")?;
    if allowed {
        let canonical = workspace::grant_path(&app, text(&q, "host_path")?)?;
        ensure!(
            canonical.to_string_lossy() == text(&q, "canonical_path")?,
            "grant_path_changed"
        );
        app.store.grant(&qid,&json!({"id":qid,"session_id":q["session_id"],"origin_job_id":q["job_id"],"origin_attempt_id":q["attempt_id"],"workspace_id":q["workspace_id"],"host_path":q["host_path"],"canonical_path":q["canonical_path"],"access":q["access"],"target":format!("/grants/{qid}"),"expires_at":now()+3600,"state":"allowed"}))?;
        if let Some(tx) = app.controls.lock().await.get(text(&q, "job_id")?) {
            let _ = tx.send("grant".into());
        }
    } else {
        app.store.settle_question(&qid, "denied")?;
        runtime
            .send(json!({"type":"extension_ui_response","id":q["rpc_id"],"confirmed":false}))
            .await?;
    }
    Ok(Json(
        json!({"state":if allowed{"allowed"}else{"denied"},"target":if allowed{Some(format!("/grants/{qid}"))}else{None},"next_turn_required":allowed}),
    ))
}
async fn revoke(
    State(app): State<Arc<App>>,
    Extension(p): Extension<Principal>,
    Path((wid, gid)): Path<(String, String)>,
) -> Reply {
    owner(&p)?;
    let _lock = app.admission.lock().await;
    own(&app, &p, "workspace", &wid)?;
    let w = app.store.change("workspace", &wid, |w| {
        w["grants"]
            .as_array_mut()
            .context("grants_invalid")?
            .retain(|g| g["id"] != gid);
        w["generation"] = json!(w["generation"].as_u64().unwrap_or(0) + 1);
        Ok(())
    })?;
    stop_matching(&app, None, None, Some(&wid), "revoke").await?;
    Ok(Json(w))
}
async fn stop_matching(
    app: &App,
    provider: Option<&str>,
    model: Option<&str>,
    workspace: Option<&str>,
    reason: &str,
) -> Result<()> {
    for j in app.store.list("job", None)? {
        if store::terminal(text(&j, "state")?) {
            continue;
        }
        let s = app
            .store
            .get("session", text(&j, "session_id")?)?
            .context("session_missing")?;
        if provider.is_none_or(|p| s["provider_id"] == p)
            && model.is_none_or(|m| s["model_id"] == m)
            && workspace.is_none_or(|w| s["workspace_id"] == w)
        {
            app.store.change("job", text(&j, "id")?, |v| {
                v["state"] = json!("cancelling");
                Ok(())
            })?;
            if let Some(tx) = app.controls.lock().await.get(text(&j, "id")?) {
                let _ = tx.send(reason.into());
            }
        }
    }
    Ok(())
}
