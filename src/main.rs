use axum_server::Handle;
use clap::{Parser, Subcommand};
use std::{net::SocketAddr, sync::Arc};

mod config;
mod container;
mod features;
mod fs;
mod metrics;
mod poller;
mod routes;
mod tracing_setup;

use container::{
    DeploymentOrchestrator, docker::DockerClient, github::GitHubClient,
};
use routes::AppState;

#[derive(Parser)]
#[command(version)]
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

    let config = config::Config::load().unwrap_or_else(|e| {
        eprintln!("Configuration error: {e}");
        std::process::exit(1);
    });
    let config = Arc::new(config);

    let tracer_provider = tracing_setup::init_tracing(&config);

    match cli.command {
        Commands::Serve => serve(cli.host, cli.port, config, tracer_provider).await,
    }
}

async fn serve(
    host: String,
    port: u16,
    config: Arc<config::Config>,
    tracer_provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
) {
    let host: std::net::IpAddr = host.parse().unwrap_or_else(|_| {
        eprintln!("Invalid host address: '{host}'");
        std::process::exit(1);
    });
    let addr = SocketAddr::new(host, port);

    let flags = features::new_flags();

    // New logic
    let github = Arc::new(GitHubClient::new(config.github_token.clone()));

    let docker = DockerClient::new().await.unwrap_or_else(|e| {
        eprint!("Failed to init Docker client: {e}");
        std::process::exit(1);
    });

    let orchestrator = DeploymentOrchestrator::with_executor(
        Arc::clone(&config),
        Arc::clone(&github),
        flags.clone(),
        docker,
    )
    .await
    .unwrap_or_else(|e| {
        eprintln!("Failed to init orchestrator: {e}");
        std::process::exit(1);
    });

    let state = AppState { config, orchestrator: Arc::new(orchestrator), flags };

    match &state.config.mode {
        config::Mode::Poll { repo, interval_secs, .. } => {
            let p = poller::Poller::new(
                Arc::clone(&state.orchestrator),
                Arc::clone(&github),
                state.flags.clone(),
                Arc::clone(&state.config),
            );
            tracing::info!(repo = %repo, interval_secs, "Polling mode active");
            tokio::spawn(async move { p.run().await });
        }
        config::Mode::Webhook { .. } => {}
    }

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "shepherd starting");
    tracing::info!("Starting shepherd on {addr}");

    if state.config.initial_sync {
        let orchestrator = Arc::clone(&state.orchestrator);
        tokio::spawn(async move {
            if let Err(e) = orchestrator.initial_sync().await {
                tracing::error!("Initial sync failed: {e:?}");
            }
        });
    }

    let app = routes::router(state);
    let handle: Handle<SocketAddr> = Handle::new();
    tokio::spawn(shutdown_signal(handle.clone()));

    let result =
        axum_server::bind(addr).handle(handle).serve(app.into_make_service()).await;

    // Flush telemetry regardless of how the server exits.
    if let Some(provider) = tracer_provider
        && let Err(e) = provider.shutdown()
    {
        tracing::warn!("Failed to shutdown tracer provider: {e}");
    }

    if let Err(e) = result {
        tracing::error!("Server error: {e}");
        std::process::exit(1);
    }
}

async fn shutdown_signal(handle: Handle<SocketAddr>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received, starting graceful shutdown");
    handle.graceful_shutdown(Some(std::time::Duration::from_secs(30)));
}
