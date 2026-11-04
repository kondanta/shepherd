# syntax=docker/dockerfile:1@sha256:4edf897a3ffa55b89f906fc8cc78afdb3f1834cc9c7083565e611a8a7d5fe99e

FROM lukemathwalker/cargo-chef:latest-rust-alpine@sha256:000cac9096459ebc31e55e1a99bed090b1ce3511f006b4b101fd2d4f4bcb4d1e AS chef
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

FROM docker:cli@sha256:4d742ab934c2149b628bf26369ac3ec51cb91e2e28fe6722a0e86a6d6efc4bd5

COPY --from=builder /app/target/release/shepherd /usr/local/bin/shepherd

# Shepherd does not open any ports itself — axum listens on the port
# configured via --port (default 8080). Declare it for documentation.
EXPOSE 8080

ENTRYPOINT ["shepherd"]
CMD ["serve"]
