use axum::{
    Router,
    middleware,
    routing::{get, post},
};
use clap::{Parser, Subcommand};
use std::{net::SocketAddr, sync::Arc};

mod config;
mod container;
mod features;
mod fs;
mod routes;
mod tracing_setup;

use routes::{AppState, make_orchestrator};

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
    let config = config::Config::load();
    let shared_config = Arc::new(config);
    let tracer_provider = tracing_setup::init_tracing(&shared_config);
    let flags = features::new_flags();

    let res = match &cli.command {
        Commands::Serve => {
            tracing::info!("Starting shepherd server on {}:{}", cli.host, cli.port);
            let addr = SocketAddr::new(cli.host.parse().unwrap(), cli.port);
            let orchestrator = make_orchestrator(&shared_config, flags.clone());
            let state = AppState {
                config: shared_config.clone(),
                orchestrator,
                flags,
            };
            let flags_router = Router::new()
                .route("/flags", get(crate::routes::get_flags))
                .route("/flags/pause", post(crate::routes::pause_deployments))
                .route("/flags/resume", post(crate::routes::resume_deployments))
                .route(
                    "/flags/dry-run/enable",
                    post(crate::routes::enable_dry_run),
                )
                .route(
                    "/flags/dry-run/disable",
                    post(crate::routes::disable_dry_run),
                )
                .layer(middleware::from_fn_with_state(
                    state.clone(),
                    crate::routes::require_api_token,
                ));

            if state.config.api_token.is_none() {
                tracing::warn!(
                    "API_TOKEN is not set; /flags/* endpoints will return 401"
                );
            }

            let mut app = Router::new()
                .route("/", get(crate::routes::root))
                .route("/health", get(crate::routes::health_check))
                .route("/list-services", get(crate::routes::list_managed_services))
                .route("/webhook/github", post(crate::routes::github_webhook))
                .route("/deployments", get(crate::routes::list_deployments))
                .route("/deploy", post(crate::routes::manual_deploy))
                .merge(flags_router)
                .with_state(state);
            app = add_feature_routes(app);
            axum_server::bind(addr).serve(app.into_make_service()).await
        }
    };

    if let Err(e) = res {
        eprintln!("Server error: {e}");
        std::process::exit(1);
    }

    if let Some(provider) = tracer_provider {
        provider
            .shutdown()
            .expect("Failed to shutdown tracer provider");
    }
}

#[allow(unused_mut)]
fn add_feature_routes(mut app: Router) -> Router {
    #[cfg(feature = "otlp")]
    {
        app = app.route("/metrics", axum::routing::get(routes::metrics));
    }
    app
}
