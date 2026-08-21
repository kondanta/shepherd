# syntax=docker/dockerfile:1@sha256:ecfaec9ed6d810b56388c508f4121597bfbba70d41a6dfeee4d8cad5f295fc32

FROM lukemathwalker/cargo-chef:latest-rust-alpine@sha256:917b051d1fc8e234a3aad123378b5263c95fa5d8739439ee25aa789c2db97a90 AS chef
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
