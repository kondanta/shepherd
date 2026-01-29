use axum::{Json, extract::State, http::StatusCode};
use std::sync::Arc;
// use axum_macros::debug_handler;

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

#[cfg_attr(feature = "otlp", instrument(skip(config)))]
pub async fn scan_filesystem(
    State(config): State<Arc<Config>>,
) -> Json<DummyResponse> {
    let root_path = std::path::Path::new(&config.root_dir);
    let scan_results = f::walk::scan_filesystem(root_path).unwrap_or_default();

    tracing::info!(
        "Scanned filesystem at {:?}, found {} services",
        root_path,
        scan_results.len()
    );

    let response = DummyResponse {
        results: scan_results,
    };

    Json(response)
}
