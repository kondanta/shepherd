use crate::{Cli, Commands};

#[cfg(feature = "otlp")]
mod otlp_imports {
    pub use opentelemetry_otlp::SpanExporter;
    pub use opentelemetry_sdk::Resource;
    pub use opentelemetry_sdk::trace::SdkTracerProvider;
    pub use std::sync::OnceLock;
}

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
pub fn init_trace() -> SdkTracerProvider {
    let exporter = SpanExporter::builder()
        .with_tonic() // Use gRPC protocol. Use .with_http() for HTTP/protobuf or .with_http_json() for HTTP/JSON
        .build()
        .expect("Failed to create OTLP trace exporter");
    SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(get_resource())
        .build()
}

pub fn init_logs(opt: &Cli) {
    if std::env::var("RUST_LOG").is_err() {
        let default_log = match &opt.command {
            Commands::Serve => "shepherd=info,tower_http=info",
        };
        unsafe {
            std::env::set_var("RUST_LOG", default_log);
        }
    }

    let env_filter = tracing_subscriber::filter::EnvFilter::from_default_env();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(env_filter)
        .init();
}
