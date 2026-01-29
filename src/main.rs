use axum::{Router, routing::get};
use axum_server;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;

#[cfg(feature = "otlp")]
use opentelemetry_sdk;

mod config;
mod features;
mod routes;
mod tracing_setup;

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value_t = 8080)]
    port: u16,

    #[arg(long, default_value = "0.0.0.0")]
    host: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Serve,
}

#[tokio::main]
async fn main() {
    color_eyre::install().expect("Failed to install color_eyre");
    let cli = Cli::parse();
    let features = &[
        #[cfg(feature = "otlp")]
        "otlp",
    ][..];
    features::set_features(features.to_vec());

    let tracer_provider = init_tracing(&cli);

    let res = match &cli.command {
        Commands::Serve => {
            tracing::info!("Starting shepherd server on {}:{}", cli.host, cli.port);
            let addr = SocketAddr::new(cli.host.parse().unwrap(), cli.port);
            // #[allow(unused_mut)]
            let mut app = Router::new()
                .route("/", get(crate::routes::root()))
                .route("/health", get(crate::routes::health_check));
            app = add_feature_routes(app);
            axum_server::bind(addr).serve(app.into_make_service()).await
        }
    };

    if let Err(e) = res {
        eprintln!("Server error: {}", e);
        std::process::exit(1);
    }

    if let Some(tracer_provider) = tracer_provider {
        tracer_provider
            .shutdown()
            .expect("Failed to shutdown tracer provider");
    }
}

#[allow(unused_mut)] // as it is used in conditional compilation
fn add_feature_routes(mut app: Router) -> Router {
    #[cfg(feature = "otlp")]
    {
        app = app.route("/metrics", axum::routing::get(routes::metrics));
    }
    app
}

#[cfg(feature = "otlp")]
fn init_tracing(cli: &Cli) -> Option<opentelemetry_sdk::trace::SdkTracerProvider> {
    let _otlp_endpoint = std::env::var("OTLP_ENDPOINT")
        .expect("OTLP_ENDPOINT environment variable not set");
    let tracer_provider = tracing_setup::init_trace();
    opentelemetry::global::set_tracer_provider(tracer_provider.clone());
    tracing_setup::init_logs(&cli);

    Some(tracer_provider)
}

#[cfg(not(feature = "otlp"))]
fn init_tracing(cli: &Cli) -> Option<opentelemetry_sdk::trace::SdkTracerProvider> {
    // No-op if OTLP feature is not enabled
    // Just initialize logs
    tracing_setup::init_logs(&cli);
    None
}
