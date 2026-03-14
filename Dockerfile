# syntax=docker/dockerfile:1

# ── Dependency planner ────────────────────────────────────────────────────────
# cargo-chef analyses the dependency graph and produces a recipe.json.
# This layer is only invalidated when Cargo.toml or Cargo.lock change,
# keeping the expensive dependency compile step cached across source changes.
FROM lukemathwalker/cargo-chef:latest-rust-alpine AS chef
RUN apk add --no-cache musl-dev
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ── Dependency compile ────────────────────────────────────────────────────────
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --features metrics --recipe-path recipe.json

# ── Application compile ───────────────────────────────────────────────────────
COPY . .
RUN cargo build --release --features metrics

# ── Runtime ───────────────────────────────────────────────────────────────────
# docker:cli is the canonical Alpine-based image that includes both the
# docker CLI and the 'docker compose' plugin. No host tooling is required
# other than the Docker daemon (exposed via the mounted socket).
FROM docker:cli

COPY --from=builder /app/target/release/shepherd /usr/local/bin/shepherd

# Shepherd does not open any ports itself — axum listens on the port
# configured via --port (default 8080). Declare it for documentation.
EXPOSE 8080

ENTRYPOINT ["shepherd"]
CMD ["serve"]
