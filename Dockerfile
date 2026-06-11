# syntax=docker/dockerfile:1@sha256:87999aa3d42bdc6bea60565083ee17e86d1f3339802f543c0d03998580f9cb89

FROM lukemathwalker/cargo-chef:latest-rust-alpine@sha256:7447ff76d65e123f0b53c339155dffa666bb742b6866b0d61f35beab5514a3a1 AS chef
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

FROM docker:cli@sha256:873de13208aab9c1de73fe984fd45883e01464fcfcc85efa20aa56a9ccfe7aa6

COPY --from=builder /app/target/release/shepherd /usr/local/bin/shepherd

# Shepherd does not open any ports itself — axum listens on the port
# configured via --port (default 8080). Declare it for documentation.
EXPOSE 8080

ENTRYPOINT ["shepherd"]
CMD ["serve"]
