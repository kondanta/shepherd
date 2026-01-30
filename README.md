# Shepherd

**Shepherd** is a lightweight automation tool that keeps Docker Compose–based workloads up to date when container images change.

It listens for GitHub webhook events (e.g. from Renovate), scans a filesystem for Docker Compose files, detects which service reference updated images, updates them in place, and restarts only the affected services.

Shepherd is designed for **self-hosted environments** and **homelabs**, not as a general-purpose container platform.


---

## Status

🚧 **Early development**

Core functionality (scanning, parsing, observability) is in place.
Webhook handling and Docker Compose execution are evolving.


---

## Why Shepherd?

If you:

* run **multiple Docker Compose workloads** on a single host
* use **Renovate** to keep images updated
* want **controlled, observable restarts** without moving to Kubernetes

…Shepherd bridges that gap.

---

## How It Works

1. Renovate updates Docker image versions in a repository
2. GitHub sends a webhook event
3. Shepherd:

   * scans the configured root directory for `docker-compose.yaml` / `.yml` files
   * parses services and image references
   * identifies services using updated images
   * updates the image fields in place
   * restarts only the affected services using `docker compose`
4. Metrics and traces are emitted for observability

---

## Features

* 🔍 Recursive discovery of Docker Compose files
* 🐳 Service and image extraction
* ✏️ In-place image updates
* ▶️ Targeted service restarts (no full stack restarts)
* 📊 Metrics:

  * services restarted
  * successful updates
  * failed updates
  * scan errors
* 🧾 Structured logging via `tracing`
* 🔎 Optional OpenTelemetry tracing

---

## Assumptions & Constraints

Shepherd is intentionally opinionated:

* **Only supports `docker compose`**

  * `docker-compose` (v1) is **not supported**
* **Filesystem-based**

  * No Docker API orchestration
* **Single-host focus**

  * Not a cluster scheduler
* **Idempotent by design**

  * Re-running does not cause unnecessary restarts

---

## Configuration

Configuration is environment-based.
A `.env` file is optional but supported.

### Required

* `ROOT_DIR`
  Root directory to scan for Docker Compose files.

### Optional

* `LOG_LEVEL`
  Log level (`info` by default).

* `OTLP_ENDPOINT`
  Required **only** when built with the `otlp` feature.

Example:

```env
ROOT_DIR=/srv/compose
LOG_LEVEL=info
OTLP_ENDPOINT=http://localhost:4317
```

---

## Observability

### Logging

* Powered by `tracing`
* Default log level: `info`
* Noisy dependencies are filtered out

### Tracing (Optional)

When built with the `otlp` feature:

* Traces are exported using **OTLP over gRPC**
* HTTP-based OTLP exporters are **not supported**
* Intended for backends like **Grafana Tempo**

Build with tracing enabled:

```bash
cargo run --features otlp -- serve
```

---

## Endpoints

* `GET /scan`
  Scans the filesystem and reports discovered services.

* `GET /metrics`
  Exposes runtime metrics (available when OTLP is enabled).

---

## Non-Goals

Shepherd is **not**:

* a replacement for Kubernetes
* a UI-driven container manager
* a multi-host orchestrator

If you want those, use Portainer, Komodo or Kubernetes.

If you want **simple, observable automation for Docker Compose**, Shepherd exists.
