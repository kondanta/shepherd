//! Thin wrappers around the `metrics` facade crate.
//!
//! The `metrics` crate is always compiled but is a no-op unless a recorder
//! is installed. With the `otlp` feature the `axum-prometheus` layer installs
//! a Prometheus recorder at startup. Without the feature every call here
//! compiles to nothing at the call site.
//!
//! Label values that come from user data (service name) are `String`; static
//! disposition/status strings use `&'static str` so they are interned by the
//! metrics crate without allocation.

pub(crate) fn webhook_received(disposition: &'static str) {
    metrics::counter!(
        "shepherd_webhooks_total",
        "disposition" => disposition
    )
    .increment(1);
}

pub(crate) fn deployment_recorded(
    service: &str,
    success: bool,
    duration_secs: f64,
) {
    let status = if success { "success" } else { "failed" };
    metrics::counter!(
        "shepherd_deployments_total",
        "service" => service.to_owned(),
        "status" => status
    )
    .increment(1);

    if success {
        metrics::histogram!(
            "shepherd_deployment_duration_seconds",
            "service" => service.to_owned()
        )
        .record(duration_secs);
    }
}

pub(crate) fn set_deployments_paused(paused: bool) {
    metrics::gauge!("shepherd_deployments_paused").set(if paused { 1.0 } else { 0.0 });
}

pub(crate) fn set_dry_run(enabled: bool) {
    metrics::gauge!("shepherd_dry_run_enabled").set(if enabled { 1.0 } else { 0.0 });
}

pub(crate) fn set_managed_services(count: usize) {
    metrics::gauge!("shepherd_managed_services_total").set(count as f64);
}
