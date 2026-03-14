use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::Config;
use crate::container::{
    self, Deployment, DeploymentOrchestrator, webhook::WebhookEvent,
};
use crate::features::SharedFlags;

#[cfg(feature = "otlp")]
mod otlp_imports {
    pub use opentelemetry::{
        KeyValue,
        trace::{TraceContextExt, Tracer},
    };
}

#[cfg(feature = "otlp")]
use otlp_imports::*;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub orchestrator: Arc<Mutex<DeploymentOrchestrator>>,
    pub flags: SharedFlags,
}

pub fn make_orchestrator(
    config: &Config,
    flags: SharedFlags,
) -> Arc<Mutex<DeploymentOrchestrator>> {
    let orch = DeploymentOrchestrator::new(config, flags)
        .expect("Failed to initialize DeploymentOrchestrator");
    Arc::new(Mutex::new(orch))
}

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

pub(crate) async fn root() -> &'static str {
    tracing::info!("Root endpoint was called");
    "Shepherd is running!"
}

pub(crate) async fn health_check() -> StatusCode {
    StatusCode::OK
}

#[cfg(feature = "otlp")]
pub(crate) async fn metrics() -> Json<&'static str> {
    let tracer = opentelemetry::global::tracer("shepherd-metrics");
    tracer.in_span("metrics_endpoint", |cx| {
        let span = cx.span();
        span.set_attribute(KeyValue::new("endpoint", "/metrics"));
        tracing::info!("Metrics endpoint was called");
    });
    Json("metrics data")
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct ManagedServicesResponse {
    pub services: Vec<crate::fs::walk::ServiceEntry>,
    pub total: usize,
}

pub async fn list_managed_services(
    State(state): State<AppState>,
) -> Result<Json<ManagedServicesResponse>, StatusCode> {
    let orch = state.orchestrator.lock().await;
    let services = orch.get_managed_services().map_err(|e| {
        tracing::error!("Failed to get managed services: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let total = services.len();
    Ok(Json(ManagedServicesResponse { services, total }))
}

pub async fn github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let signature = match headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s.to_owned(),
        None => {
            tracing::warn!("Missing X-Hub-Signature-256 header");
            return StatusCode::UNAUTHORIZED;
        }
    };

    if !verify_github_signature(&state.config.webhook_secret, &body, &signature) {
        tracing::warn!("Invalid webhook signature");
        return StatusCode::UNAUTHORIZED;
    }

    let event = match headers.get("X-GitHub-Event").and_then(|v| v.to_str().ok()) {
        Some(e) => WebhookEvent::from_header(e),
        None => {
            tracing::warn!("Missing X-GitHub-Event header");
            return StatusCode::BAD_REQUEST;
        }
    };

    if event != WebhookEvent::Push {
        tracing::debug!("Ignoring non-push event: {event:?}");
        return StatusCode::OK;
    }

    if state.flags.load().deployments_paused {
        tracing::info!("Deployments paused, ignoring webhook");
        return StatusCode::OK;
    }

    let payload =
        match serde_json::from_slice::<container::webhook::WebhookPayload>(&body) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to parse webhook payload: {:?}", e);
                return StatusCode::BAD_REQUEST;
            }
        };

    let mut orch = state.orchestrator.lock().await;
    if let Err(e) = orch.handle_webhook(&payload).await {
        tracing::error!("Webhook handling failed: {:?}", e);
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    StatusCode::OK
}

pub async fn list_deployments(
    State(state): State<AppState>,
) -> Json<Vec<Deployment>> {
    let orch = state.orchestrator.lock().await;
    Json(orch.list_deployments())
}

#[derive(serde::Deserialize)]
pub struct ManualDeployRequest {
    pub service: String,
}

pub async fn manual_deploy(
    State(state): State<AppState>,
    Json(req): Json<ManualDeployRequest>,
) -> StatusCode {
    if state.flags.load().deployments_paused {
        tracing::info!("Deployments paused, rejecting manual deploy");
        return StatusCode::SERVICE_UNAVAILABLE;
    }

    let mut orch = state.orchestrator.lock().await;
    match orch.deploy_service_by_name(&req.service).await {
        Ok(()) => StatusCode::OK,
        Err(e) => {
            tracing::error!("Manual deploy of '{}' failed: {:?}", req.service, e);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[derive(serde::Serialize)]
pub struct FlagsResponse {
    pub deployments_paused: bool,
    pub dry_run: bool,
}

fn unauthorized(reason: &str) -> Response {
    tracing::warn!("Unauthorized /flags access: {}", reason);
    let body = serde_json::json!({ "error": "unauthorized", "reason": reason });
    (
        StatusCode::UNAUTHORIZED,
        [("content-type", "application/json")],
        body.to_string(),
    )
        .into_response()
}

pub async fn require_api_token(
    State(state): State<AppState>,
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
        Some(token) if token == expected => next.run(req).await,
        Some(_) => unauthorized("invalid token"),
        None => unauthorized("missing Authorization header"),
    }
}

pub async fn get_flags(State(state): State<AppState>) -> Json<FlagsResponse> {
    let f = state.flags.load();
    Json(FlagsResponse {
        deployments_paused: f.deployments_paused,
        dry_run: f.dry_run,
    })
}

pub async fn pause_deployments(State(state): State<AppState>) -> StatusCode {
    state.flags.rcu(|f| crate::features::RuntimeFlags {
        deployments_paused: true,
        ..(**f).clone()
    });
    tracing::info!("Deployments paused");
    StatusCode::OK
}

pub async fn resume_deployments(State(state): State<AppState>) -> StatusCode {
    state.flags.rcu(|f| crate::features::RuntimeFlags {
        deployments_paused: false,
        ..(**f).clone()
    });
    tracing::info!("Deployments resumed");
    StatusCode::OK
}

pub async fn enable_dry_run(State(state): State<AppState>) -> StatusCode {
    state.flags.rcu(|f| crate::features::RuntimeFlags {
        dry_run: true,
        ..(**f).clone()
    });
    tracing::info!("Dry-run mode enabled");
    StatusCode::OK
}

pub async fn disable_dry_run(State(state): State<AppState>) -> StatusCode {
    state.flags.rcu(|f| crate::features::RuntimeFlags {
        dry_run: false,
        ..(**f).clone()
    });
    tracing::info!("Dry-run mode disabled");
    StatusCode::OK
}
