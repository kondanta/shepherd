use axum::{Json, http::StatusCode};
// use axum_macros::debug_handler;

#[cfg(feature = "otlp")]
use opentelemetry::{
    KeyValue,
    trace::{TraceContextExt, Tracer},
};

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
