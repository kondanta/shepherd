use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::{filter::EnvFilter, prelude::*};

#[cfg(feature = "otlp")]
mod otlp_imports {
    pub use opentelemetry::trace::TracerProvider as _;
    pub use opentelemetry_otlp::SpanExporter;
    pub use opentelemetry_otlp::WithExportConfig;
    pub use opentelemetry_sdk::Resource;
    pub use std::sync::OnceLock;
}

use crate::config::Config;

#[cfg(feature = "otlp")]
use otlp_imports::*;

#[cfg(feature = "otlp")]
fn get_resource() -> Resource {
    static RESOURCE: OnceLock<Resource> = OnceLock::new();
    RESOURCE
        .get_or_init(|| Resource::builder().with_service_name("shepherd").build())
        .clone()
}

#[cfg(feature = "otlp")]
pub fn init_tracing(config: &Config) -> Option<SdkTracerProvider> {
    let exporter = match SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&config.otlp_endpoint)
        .build()
    {
        Ok(e) => e,
        Err(e) => {
            eprintln!(
                "Warning: failed to create OTLP trace exporter ({e}); \
                 falling back to stderr logging"
            );
            init_stderr_logging(&config.log_level);
            return None;
        }
    };

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(get_resource())
        .build();

    let tracer = provider.tracer("shepherd-tracer");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::NONE);

    tracing_subscriber::registry()
        .with(set_env_filter(&config.log_level))
        .with(otel_layer)
        .with(fmt_layer)
        .try_init()
        .expect("Failed to initialize tracing subscriber");

    opentelemetry::global::set_tracer_provider(provider.clone());

    Some(provider)
}

#[cfg(not(feature = "otlp"))]
pub fn init_tracing(config: &Config) -> Option<SdkTracerProvider> {
    init_stderr_logging(&config.log_level);
    None
}

fn init_stderr_logging(log_level: &str) {
    tracing_subscriber::registry()
        .with(set_env_filter(log_level))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();
}

fn set_env_filter(log_level: &str) -> EnvFilter {
    // Shepherd's own log level is controlled by LOG_LEVEL.
    // Framework crates are capped at `info` so debug/trace doesn't flood logs.
    EnvFilter::new(format!("shepherd={log_level}"))
        .add_directive("tower_http=info".parse().unwrap())
        .add_directive("axum_server=info".parse().unwrap())
        .add_directive("hyper=off".parse().unwrap())
        .add_directive("tonic=off".parse().unwrap())
        .add_directive("h2=off".parse().unwrap())
        .add_directive("reqwest=off".parse().unwrap())
}
