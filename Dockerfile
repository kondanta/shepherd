# syntax=docker/dockerfile:1@sha256:4a43a54dd1fedceb30ba47e76cfcf2b47304f4161c0caeac2db1c61804ea3c91

FROM lukemathwalker/cargo-chef:latest-rust-alpine@sha256:5b2b5c6585c537a2795a477e93ebba85b4a2887e11ee9bddd34ad607e53ccec0 AS chef
RUN apk add --no-cache musl-dev
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --features metrics --recipe-path recipe.json

COPY . .
RUN cargo build --release --features metrics

FROM docker:cli@sha256:0befd75049d8903cd5606879d251bd29ac08758cd33f9c7c74583fc6679b737d

COPY --from=builder /app/target/release/shepherd /usr/local/bin/shepherd

# Shepherd does not open any ports itself — axum listens on the port
# configured via --port (default 8080). Declare it for documentation.
EXPOSE 8080

ENTRYPOINT ["shepherd"]
CMD ["serve"]
