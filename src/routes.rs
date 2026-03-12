use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

use crate::container::{self, Deployment, DeploymentOrchestrator};
use crate::{config::Config, fs as f};

#[cfg(feature = "otlp")]
mod otlp_imports {
    pub use opentelemetry::{
        KeyValue,
        trace::{TraceContextExt, Tracer},
    };
    pub use tracing::instrument;
}

#[cfg(feature = "otlp")]
use otlp_imports::*;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub orchestrator: Arc<Mutex<DeploymentOrchestrator>>,
}

pub fn make_orchestrator(config: &Config) -> Arc<Mutex<DeploymentOrchestrator>> {
    let orch = DeploymentOrchestrator::new(
        PathBuf::from(&config.root_dir),
        Some("".to_string()),
    )
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

pub(crate) fn root() -> &'static str {
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

#[derive(serde::Serialize)]
pub struct DummyResponse {
    pub results: Vec<f::walk::ServiceEntry>,
}

#[cfg_attr(feature = "otlp", instrument(skip(state)))]
pub async fn scan_filesystem(State(state): State<AppState>) -> Json<DummyResponse> {
    let root_path = std::path::Path::new(&state.config.root_dir);
    let scan_results = f::walk::scan_filesystem(root_path).unwrap_or_default();

    tracing::info!(
        "Scanned filesystem at {:?}, found {} services",
        root_path,
        scan_results.len()
    );

    Json(DummyResponse {
        results: scan_results,
    })
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

    let payload =
        match serde_json::from_slice::<container::webhook::WebhookPayload>(&body) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Failed to parse webhook payload: {:?}", e);
                return StatusCode::BAD_REQUEST;
            }
        };

    let mut orch = state.orchestrator.lock().await;
    if let Err(e) = orch
        .handle_webhook(
            &payload,
            &state.config.renovate_username,
            &state.config.renovate_email,
        )
        .await
    {
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
    let mut orch = state.orchestrator.lock().await;
    match orch.deploy_service_by_name(&req.service).await {
        Ok(()) => StatusCode::OK,
        Err(e) => {
            tracing::error!("Manual deploy of '{}' failed: {:?}", req.service, e);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
