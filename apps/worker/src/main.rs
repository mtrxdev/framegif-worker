use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use framegif_worker::blob::ObjectStore;
use framegif_worker::encode::FfmpegGifski;
use framegif_worker::hmac_auth::{unix_now, verify_framegif_signature, verify_qstash_signature};
use framegif_worker::jobs::{Ffprobe, JobContext, run_job};
use framegif_worker::types::CreateJobRequest;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct AppState {
    secret: String,
    qstash_current: Option<String>,
    qstash_next: Option<String>,
    web_url: Option<String>,
    public_url: String,
    store: Arc<ObjectStore>,
    client: reqwest::Client,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let secret = std::env::var("JOB_SECRET").unwrap_or_else(|_| "dev-secret".into());
    let state = AppState {
        secret,
        qstash_current: std::env::var("QSTASH_CURRENT_SIGNING_KEY").ok(),
        qstash_next: std::env::var("QSTASH_NEXT_SIGNING_KEY").ok(),
        web_url: std::env::var("FRAMEGIF_WEB_URL").ok(),
        public_url: std::env::var("WORKER_PUBLIC_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".into()),
        store: Arc::new(ObjectStore::from_env()),
        client: reqwest::Client::new(),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/jobs", post(create_job))
        .route("/uploads", post(create_upload).options(preflight))
        .route("/blob/{*pathname}", get(serve_blob).options(preflight))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = listen_addr();
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind worker");
    tracing::info!("framegif worker listening on {addr}");
    axum::serve(listener, app).await.expect("serve");
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true, "gifski": "1.34.0" }))
}

async fn create_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Err(err) = authorize(&state, &headers, &body) {
        return (StatusCode::UNAUTHORIZED, err.to_string()).into_response();
    }
    let request: CreateJobRequest = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(err) => {
            return (StatusCode::BAD_REQUEST, format!("invalid job: {err}")).into_response();
        }
    };
    let job_id = request.job_id.clone();
    let response_id = job_id.clone();
    tokio::spawn(async move {
        let ctx = JobContext {
            encoder: FfmpegGifski::default(),
            prober: Ffprobe,
            store: state.store.as_ref().clone(),
            client: state.client.clone(),
            web_url: request.callback_url.clone().or(state.web_url.clone()),
            job_secret: state.secret.clone(),
        };
        if let Err(err) = run_job(&ctx, request).await {
            tracing::error!(job_id, error = %err, "job failed");
        }
    });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "id": response_id })),
    )
        .into_response()
}

fn listen_addr() -> String {
    if let Ok(addr) = std::env::var("LISTEN_ADDR")
        && !addr.is_empty()
    {
        return addr;
    }
    if let Ok(port) = std::env::var("PORT")
        && !port.is_empty()
    {
        return format!("0.0.0.0:{port}");
    }
    "0.0.0.0:8080".into()
}

fn with_cors<T: IntoResponse>(response: T) -> impl IntoResponse {
    let mut response = response.into_response();
    let headers = response.headers_mut();
    headers.insert("access-control-allow-origin", "*".parse().unwrap());
    headers.insert(
        "access-control-allow-headers",
        "content-type, x-timestamp, x-signature, x-filename"
            .parse()
            .unwrap(),
    );
    headers.insert(
        "access-control-allow-methods",
        "GET, POST, OPTIONS".parse().unwrap(),
    );
    response
}

async fn preflight() -> impl IntoResponse {
    with_cors(StatusCode::NO_CONTENT)
}

async fn create_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let filename = header(&headers, "x-filename").unwrap_or("source");
    if let Err(err) = verify_framegif_signature(
        &state.secret,
        header(&headers, "x-timestamp"),
        header(&headers, "x-signature"),
        filename.as_bytes(),
        unix_now(),
    ) {
        return with_cors((StatusCode::UNAUTHORIZED, err.to_string())).into_response();
    }
    if body.is_empty() {
        return with_cors((StatusCode::BAD_REQUEST, "file is required".to_string()))
            .into_response();
    }
    if body.len() > 512 * 1024 * 1024 {
        return with_cors((
            StatusCode::PAYLOAD_TOO_LARGE,
            "file is too large".to_string(),
        ))
        .into_response();
    }
    let safe_name = filename.replace(
        |c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '_' && c != '-',
        "-",
    );
    let safe_name = if safe_name.is_empty() {
        "source".to_string()
    } else {
        safe_name
    };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let pathname = format!("sources/{stamp}/{safe_name}");
    let content_type = header(&headers, "content-type").unwrap_or("application/octet-stream");
    match state
        .store
        .put(&state.client, &pathname, body.to_vec(), content_type)
        .await
    {
        Ok(stored) => with_cors(Json(serde_json::json!({
            "url": stored.url,
            "pathname": stored.pathname,
        })))
        .into_response(),
        Err(err) => with_cors((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())).into_response(),
    }
}

async fn serve_blob(
    State(state): State<AppState>,
    Path(pathname): Path<String>,
) -> impl IntoResponse {
    let Some(path) = state.store.local_path(&pathname) else {
        return with_cors((StatusCode::NOT_FOUND, "Not found".to_string())).into_response();
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let gif = pathname.ends_with(".gif");
            let mut response = (
                StatusCode::OK,
                [(
                    "content-type",
                    if gif {
                        "image/gif"
                    } else {
                        "application/octet-stream"
                    },
                )],
                bytes,
            )
                .into_response();
            response
                .headers_mut()
                .insert("cache-control", "private, max-age=0".parse().unwrap());
            with_cors(response).into_response()
        }
        Err(_) => with_cors((StatusCode::NOT_FOUND, "Not found".to_string())).into_response(),
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn authorize(state: &AppState, headers: &HeaderMap, body: &[u8]) -> Result<(), String> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(token) = header("upstash-signature") {
        let keys: Vec<&str> = [&state.qstash_current, &state.qstash_next]
            .into_iter()
            .filter_map(Option::as_deref)
            .collect();
        if keys.is_empty() {
            return Err("qstash signing key missing".into());
        }
        let now = unix_now();
        let mut last = String::from("qstash verify failed");
        for key in keys {
            match verify_qstash_signature(key, token, body, &state.public_url, now) {
                Ok(()) => return Ok(()),
                Err(err) => last = err.to_string(),
            }
        }
        return Err(last);
    }
    verify_framegif_signature(
        &state.secret,
        header("x-timestamp"),
        header("x-signature"),
        body,
        unix_now(),
    )
    .map_err(|err| err.to_string())
}
