use axum::{
    Json,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::sync::Arc;
use subtle::ConstantTimeEq;

use crate::config::Config;
use crate::container::{
    self, Deployment, DeploymentOrchestrator,
    docker::{DockerClient, DockerExecutor},
    webhook::WebhookEvent,
};

use crate::features::SharedFlags;

#[cfg(feature = "metrics")]
use axum_prometheus::PrometheusMetricLayer;

#[cfg(feature = "otlp")]
use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};

#[derive(Clone)]
pub struct AppState<D: DockerExecutor = DockerClient> {
    pub config: Arc<Config>,
    pub orchestrator: Arc<DeploymentOrchestrator<D>>,
    pub flags: SharedFlags,
}

/// Build the complete router. This is the single place that owns all route
/// definitions and their middleware — `main` just binds and serves.
pub fn router<D: DockerExecutor>(state: AppState<D>) -> axum::Router {
    if state.config.api_token.is_none() {
        tracing::warn!("API_TOKEN not set; /flags/* endpoints will return 401");
    }

    let flags_router = axum::Router::new()
        .route("/deploy", post(manual_deploy))
        .route("/sync", post(trigger_sync))
        .route("/deployments", get(list_deployments))
        .route("/list-services", get(list_managed_services))
        .route("/flags", get(get_flags))
        .route("/flags/pause", post(pause_deployments))
        .route("/flags/resume", post(resume_deployments))
        .route("/flags/dry-run/enable", post(enable_dry_run))
        .route("/flags/dry-run/disable", post(disable_dry_run))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_api_token,
        ));

    #[cfg(feature = "metrics")]
    let (prometheus_layer, metric_handle) = PrometheusMetricLayer::pair();

    let webhook_router = match &state.config.mode {
        crate::config::Mode::Webhook { secret } => {
            let secret = secret.clone();
            axum::Router::new().route(
                "/webhook/github",
                post(
                    move |State(st): State<AppState<D>>,
                          hdrs: HeaderMap,
                          body: Bytes| {
                        let s = secret.clone();
                        async move { github_webhook(st, hdrs, body, s).await }
                    },
                )
                .layer(DefaultBodyLimit::max(1024 * 1024)),
            )
        }
        crate::config::Mode::Poll { .. } => axum::Router::new(),
    };

    let app = axum::Router::new()
        .route("/", get(root))
        .route("/healthz", get(health_check))
        .route("/readyz", get(readiness_check))
        .merge(webhook_router)
        .merge(flags_router);

    #[cfg(feature = "metrics")]
    let app = {
        app.route("/metrics", get(move || async move { metric_handle.render() }))
            .layer(prometheus_layer)
    };

    #[cfg(feature = "otlp")]
    let app = { app.layer(OtelInResponseLayer).layer(OtelAxumLayer::default()) };

    app.with_state(state)
}

// ── middleware ────────────────────────────────────────────────────────────────

fn unauthorized(reason: &str) -> Response {
    tracing::warn!("Unauthorized /flags access: {}", reason);
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": "unauthorized", "reason": reason })),
    )
        .into_response()
}

pub async fn require_api_token<D: DockerExecutor>(
    State(state): State<AppState<D>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let Some(expected) = &state.config.api_token else {
        return unauthorized("API_TOKEN is not configured on this instance");
    };

    let provided = req
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match provided {
        Some(token) if token.as_bytes().ct_eq(expected.as_bytes()).into() => {
            next.run(req).await
        }
        Some(_) => unauthorized("invalid token"),
        None => unauthorized("missing Authorization header"),
    }
}

// ── signature verification ────────────────────────────────────────────────────

fn verify_github_signature(secret: &str, body: &[u8], signature: &str) -> bool {
    let Some(sig_hex) = signature.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(sig_bytes) = hex::decode(sig_hex) else {
        return false;
    };
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(body);
    mac.verify_slice(&sig_bytes).is_ok()
}

// ── handlers ──────────────────────────────────────────────────────────────────

pub(crate) async fn root() -> &'static str {
    "Shepherd is running!"
}

#[derive(serde::Serialize)]
pub(crate) struct HealthResponse {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct ReadyCheck {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct ReadyResponse {
    status: &'static str,
    checks: ReadyChecks,
}

#[derive(serde::Serialize)]
pub(crate) struct ReadyChecks {
    root_dir: ReadyCheck,
    docker: ReadyCheck,
}

/// Liveness probe — `/healthz`
///
/// Answers "should k8s restart this pod?". Checks only that ROOT_DIR is
/// accessible. Intentionally lightweight: no subprocess, no network calls.
pub(crate) async fn health_check<D: DockerExecutor>(
    State(state): State<AppState<D>>,
) -> (StatusCode, Json<HealthResponse>) {
    let root_dir = &state.config.root_dir;
    match tokio::fs::metadata(root_dir).await {
        Ok(m) if m.is_dir() => {
            (StatusCode::OK, Json(HealthResponse { status: "ok", reason: None }))
        }
        _ => {
            tracing::error!(
                "Liveness check failed: ROOT_DIR {root_dir:?} is not accessible"
            );
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(HealthResponse {
                    status: "error",
                    reason: Some("ROOT_DIR is not accessible".to_string()),
                }),
            )
        }
    }
}

/// Readiness probe — `/readyz`
///
/// Answers "should k8s send traffic to this pod?". Runs both checks
/// independently so the response always shows per-check detail regardless
/// of which one fails.
pub(crate) async fn readiness_check<D: DockerExecutor>(
    State(state): State<AppState<D>>,
) -> (StatusCode, Json<ReadyResponse>) {
    let (root_dir_result, docker_ok) = tokio::join!(
        tokio::fs::metadata(&state.config.root_dir),
        state.orchestrator.check_available(),
    );

    let root_dir_ok = root_dir_result.map(|m| m.is_dir()).unwrap_or(false);
    let all_ok = root_dir_ok && docker_ok;

    if !all_ok {
        tracing::warn!(root_dir_ok, docker_ok, "Readiness check failed");
    }

    let status_code =
        if all_ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };

    (
        status_code,
        Json(ReadyResponse {
            status: if all_ok { "ok" } else { "error" },
            checks: ReadyChecks {
                root_dir: ReadyCheck {
                    ok: root_dir_ok,
                    error: if root_dir_ok {
                        None
                    } else {
                        Some("ROOT_DIR is not accessible".to_string())
                    },
                },
                docker: ReadyCheck {
                    ok: docker_ok,
                    error: if docker_ok {
                        None
                    } else {
                        Some("docker compose plugin is not available".to_string())
                    },
                },
            },
        }),
    )
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct ManagedServicesResponse {
    pub services: Vec<crate::fs::walk::ServiceEntry>,
    pub total: usize,
}

pub async fn list_managed_services<D: DockerExecutor>(
    State(state): State<AppState<D>>,
) -> Result<Json<ManagedServicesResponse>, StatusCode> {
    let services = state.orchestrator.get_managed_services().await.map_err(|e| {
        tracing::error!("Failed to get managed services: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let total = services.len();
    Ok(Json(ManagedServicesResponse { services, total }))
}

async fn github_webhook<D: DockerExecutor>(
    state: AppState<D>,
    headers: HeaderMap,
    body: Bytes,
    secret: String,
) -> StatusCode {
    let signature =
        match headers.get("X-Hub-Signature-256").and_then(|v| v.to_str().ok()) {
            Some(s) => s.to_owned(),
            None => {
                tracing::warn!("Missing X-Hub-Signature-256 header");
                crate::metrics::webhook_received("signature_missing");
                return StatusCode::UNAUTHORIZED;
            }
        };

    if !verify_github_signature(&secret, &body, &signature) {
        tracing::warn!("Invalid webhook signature");
        crate::metrics::webhook_received("signature_invalid");
        return StatusCode::UNAUTHORIZED;
    }

    let event = match headers.get("X-GitHub-Event").and_then(|v| v.to_str().ok()) {
        Some(e) => WebhookEvent::from_header(e),
        None => {
            tracing::warn!("Missing X-GitHub-Event header");
            crate::metrics::webhook_received("missing_event_header");
            return StatusCode::BAD_REQUEST;
        }
    };

    if event != WebhookEvent::Push {
        tracing::debug!("Ignoring non-push event: {event:?}");
        crate::metrics::webhook_received("non_push_event");
        return StatusCode::OK;
    }

    if state.flags.load().deployments_paused {
        tracing::info!("Deployments paused, ignoring webhook");
        crate::metrics::webhook_received("paused");
        return StatusCode::OK;
    }

    let payload =
        match serde_json::from_slice::<container::webhook::WebhookPayload>(&body) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to parse webhook payload: {:?}", e);
                crate::metrics::webhook_received("parse_error");
                return StatusCode::BAD_REQUEST;
            }
        };

    // Spawn background task so the HTTP response is returned immediately.
    // The orchestrator's internal semaphore serializes concurrent deployments.
    crate::metrics::webhook_received("accepted");
    let orchestrator = Arc::clone(&state.orchestrator);
    tokio::spawn(async move {
        if let Err(e) = orchestrator.handle_webhook(&payload).await {
            tracing::error!("Webhook handling failed: {:?}", e);
        }
    });

    StatusCode::ACCEPTED
}

pub async fn list_deployments<D: DockerExecutor>(
    State(state): State<AppState<D>>,
) -> Json<Vec<Deployment>> {
    Json(state.orchestrator.list_deployments())
}

#[derive(serde::Deserialize)]
pub struct ManualDeployRequest {
    pub service: String,
    /// Override the image to deploy. If set, the compose file is updated on
    /// disk before restarting — useful for rollbacks or pinning a specific tag.
    /// If omitted, re-deploys whatever image is currently in the compose file.
    pub image: Option<String>,
}

pub async fn manual_deploy<D: DockerExecutor>(
    State(state): State<AppState<D>>,
    Json(req): Json<ManualDeployRequest>,
) -> StatusCode {
    if state.flags.load().deployments_paused {
        tracing::info!("Deployments paused, rejecting manual deploy");
        return StatusCode::SERVICE_UNAVAILABLE;
    }

    let service = match state.orchestrator.find_service(&req.service).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            tracing::warn!(
                "Manual deploy requested for unknown service '{}'",
                req.service
            );
            return StatusCode::NOT_FOUND;
        }
        Err(e) => {
            tracing::error!("Failed to scan services: {e:?}");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    let service_name = service.name.clone();
    tokio::spawn(async move {
        if let Err(e) = state.orchestrator.deploy_service(service, req.image).await {
            tracing::error!("Manual deploy of '{service_name}' failed: {e:?}");
        }
    });

    StatusCode::ACCEPTED
}

pub async fn trigger_sync<D: DockerExecutor>(
    State(state): State<AppState<D>>,
) -> StatusCode {
    if state.flags.load().deployments_paused {
        tracing::info!("Deployments paused, rejecting sync request");
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    let orchestrator = Arc::clone(&state.orchestrator);
    tokio::spawn(async move {
        if let Err(e) = orchestrator.initial_sync().await {
            tracing::error!("Sync failed: {e:?}");
        }
    });
    StatusCode::ACCEPTED
}

// ── flag endpoints ────────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct FlagsResponse {
    pub deployments_paused: bool,
    pub dry_run: bool,
}

pub async fn get_flags<D: DockerExecutor>(
    State(state): State<AppState<D>>,
) -> Json<FlagsResponse> {
    let f = state.flags.load();
    Json(FlagsResponse {
        deployments_paused: f.deployments_paused,
        dry_run: f.dry_run,
    })
}

pub async fn pause_deployments<D: DockerExecutor>(
    State(state): State<AppState<D>>,
) -> StatusCode {
    state.flags.rcu(|f| crate::features::RuntimeFlags {
        deployments_paused: true,
        ..(**f).clone()
    });
    crate::metrics::set_deployments_paused(true);
    tracing::info!("Deployments paused");
    StatusCode::OK
}

pub async fn resume_deployments<D: DockerExecutor>(
    State(state): State<AppState<D>>,
) -> StatusCode {
    state.flags.rcu(|f| crate::features::RuntimeFlags {
        deployments_paused: false,
        ..(**f).clone()
    });
    crate::metrics::set_deployments_paused(false);
    tracing::info!("Deployments resumed");
    StatusCode::OK
}

pub async fn enable_dry_run<D: DockerExecutor>(
    State(state): State<AppState<D>>,
) -> StatusCode {
    state
        .flags
        .rcu(|f| crate::features::RuntimeFlags { dry_run: true, ..(**f).clone() });
    crate::metrics::set_dry_run(true);
    tracing::info!("Dry-run mode enabled");
    StatusCode::OK
}

pub async fn disable_dry_run<D: DockerExecutor>(
    State(state): State<AppState<D>>,
) -> StatusCode {
    state
        .flags
        .rcu(|f| crate::features::RuntimeFlags { dry_run: false, ..(**f).clone() });
    crate::metrics::set_dry_run(false);
    tracing::info!("Dry-run mode disabled");
    StatusCode::OK
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
    };
    use tower::ServiceExt;

    // ── verify_github_signature ───────────────────────────────────────────────

    fn make_sig(secret: &str, body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        let mut mac =
            Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn valid_signature_passes() {
        let sig = make_sig("mysecret", b"hello");
        assert!(verify_github_signature("mysecret", b"hello", &sig));
    }

    #[test]
    fn wrong_secret_fails() {
        let sig = make_sig("mysecret", b"hello");
        assert!(!verify_github_signature("wrongsecret", b"hello", &sig));
    }

    #[test]
    fn tampered_body_fails() {
        let sig = make_sig("mysecret", b"hello");
        assert!(!verify_github_signature("mysecret", b"world", &sig));
    }

    #[test]
    fn missing_sha256_prefix_fails() {
        let sig = make_sig("mysecret", b"hello");
        // Strip the "sha256=" prefix to produce a bare hex string.
        let bare = sig.strip_prefix("sha256=").unwrap().to_string();
        assert!(!verify_github_signature("mysecret", b"hello", &bare));
    }

    #[test]
    fn non_hex_signature_fails() {
        assert!(!verify_github_signature(
            "mysecret",
            b"hello",
            "sha256=notvalidhex!!"
        ));
    }

    // ── require_api_token ─────────────────────────────────────────────────────

    /// Build a minimal AppState for middleware tests.
    ///
    /// Requires docker to be installed (DeploymentOrchestrator::new calls
    /// `docker compose version` on startup). Tests are not marked #[ignore]
    /// because docker is available in this project's CI environment.
    async fn test_state(
        api_token: Option<&str>,
    ) -> AppState<crate::container::docker::CapturingDockerExecutor> {
        use crate::{
            config::Config,
            container::{docker::CapturingDockerExecutor, github::GitHubClient},
            features,
        };
        let config = Arc::new(Config::for_test(api_token));
        let github_client = Arc::new(GitHubClient::new(None));
        let flags = features::new_flags();
        let orchestrator = crate::container::DeploymentOrchestrator::with_executor(
            Arc::clone(&config),
            github_client,
            flags.clone(),
            CapturingDockerExecutor::new(),
        )
        .await
        .expect("DeploymentOrchestrator::with_executor failed");
        AppState { config, orchestrator: Arc::new(orchestrator), flags }
    }

    fn token_test_app<D: DockerExecutor>(state: AppState<D>) -> axum::Router {
        axum::Router::new()
            .route("/", get(|| async { StatusCode::OK }))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                require_api_token,
            ))
            .with_state(state)
    }

    #[tokio::test]
    async fn valid_token_passes_middleware() {
        let state = test_state(Some("correct-token")).await;
        let app = token_test_app(state);
        let req = Request::builder()
            .uri("/")
            .header("Authorization", "Bearer correct-token")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn wrong_token_returns_401() {
        let state = test_state(Some("correct-token")).await;
        let app = token_test_app(state);
        let req = Request::builder()
            .uri("/")
            .header("Authorization", "Bearer wrong-token")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn missing_auth_header_returns_401() {
        let state = test_state(Some("correct-token")).await;
        let app = token_test_app(state);
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn no_api_token_configured_returns_401() {
        let state = test_state(None).await;
        let app = token_test_app(state);
        let req = Request::builder()
            .uri("/")
            .header("Authorization", "Bearer anything")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
}
