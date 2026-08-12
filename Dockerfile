# syntax=docker/dockerfile:1@sha256:87999aa3d42bdc6bea60565083ee17e86d1f3339802f543c0d03998580f9cb89

FROM lukemathwalker/cargo-chef:latest-rust-alpine@sha256:980619ec8e88608ad967ee0fb697ed3b3e4368acdbac71e5e85ae61f9e2f1a2a AS chef
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

FROM docker:cli@sha256:000bb62ff495f986c9f5578eb67cc2cb98b91138eda81d7762d5371eb8a497fe

COPY --from=builder /app/target/release/shepherd /usr/local/bin/shepherd

# Shepherd does not open any ports itself — axum listens on the port
# configured via --port (default 8080). Declare it for documentation.
EXPOSE 8080

ENTRYPOINT ["shepherd"]
CMD ["serve"]
