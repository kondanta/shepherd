# Shepherd

Shepherd keeps Docker Compose services up to date. When Renovate pushes an image bump to GitHub, the webhook triggers Shepherd to fetch the updated compose file at that commit SHA, write it locally, pull the new image, and restart only the affected service.

Built for self-hosted environments running Docker Compose on a single host. Not a cluster scheduler, not a UI — just a small daemon that automates the boring part of keeping images current.

## How it works

1. Renovate bumps an image version and pushes to your repo
2. GitHub sends a push webhook to Shepherd
3. Shepherd verifies the HMAC signature, checks the commit is from Renovate, fetches the changed compose files from GitHub at the exact commit SHA, and diffs them against the local copies
4. Services whose config changed are pulled and restarted with `docker compose up -d --no-deps`

Concurrent webhooks are serialized — the HTTP response is immediate, deploy work is queued. A re-delivered webhook is safe: if the file is already up to date, no restart happens.

## Requirements

- Docker Compose v2 (`docker compose`). The legacy `docker-compose` v1 is not supported.
- A GitHub repository containing your compose files, with a push webhook configured.
- Renovate (or commits that match the configured author identity).

## Modes

Shepherd runs in one of two modes. Set exactly one:

- **Webhook mode** — set `WEBHOOK_SECRET`. GitHub delivers push events to `/webhook/github` via a Cloudflare Tunnel or similar. The webhook endpoint is only registered in this mode.
- **Polling mode** — set `POLL_REPO`. Shepherd polls the GitHub API on a configurable interval. No public ingress required.

Setting both or neither is a startup error.

## Configuration

All configuration is via environment variables. A `.env` file is loaded if present.

| Variable | Required | Default | Description |
|---|---|---|---|
| `ROOT_DIR` | yes | — | Directory to scan for compose files |
| `WEBHOOK_SECRET` | one of | — | GitHub webhook secret (webhook mode) |
| `POLL_REPO` | one of | — | `owner/repo` to poll (polling mode) |
| `POLL_INTERVAL_SECS` | no | `300` | How often to poll, in seconds |
| `POLL_BRANCH` | no | `main` | Branch to poll |
| `GITHUB_TOKEN` | no | — | GitHub PAT; avoids rate limiting when fetching files |
| `RENOVATE_USERNAME` | no | `renovate[bot]` | Commit author username to match |
| `RENOVATE_EMAIL` | no | `renovate[bot]@users.noreply.github.com` | Commit author email to match |
| `REPO_PATH_PREFIX` | no | — | Only handle files under this repo path; strips the prefix when writing locally |
| `ALLOW_LATEST_IMAGES` | no | `false` | Allow services tagged `latest` to be deployed |
| `API_TOKEN` | no | — | Bearer token required for `/flags/*` endpoints |
| `LOG_LEVEL` | no | `info` | One of `trace`, `debug`, `info`, `warn`, `error` |
| `OTLP_ENDPOINT` | no | — | gRPC endpoint for traces; required with `--features otlp` |

## Running

```bash
shepherd serve --port 8080
```

**Docker:** Shepherd needs the Docker socket and your compose files. The compose files must be mounted at the same absolute path they have on the host — `ROOT_DIR` is used both to find files and as the working directory for `docker compose` commands.

```yaml
services:
  shepherd:
    image: ghcr.io/yourusername/shepherd:latest
    restart: unless-stopped
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
      - /srv/compose:/srv/compose
    environment:
      ROOT_DIR: /srv/compose
      WEBHOOK_SECRET: your-secret
```

See `docker-compose.example.yml` for a complete example.

## API

**Webhook** *(webhook mode only)*

`POST /webhook/github` — receives GitHub push events. Requires a valid `X-Hub-Signature-256` header. This endpoint is not registered in polling mode.

**Services**

`GET /list-services` — lists all services discovered under `ROOT_DIR`.

`GET /deployments` — deployment history, newest first, capped at 200 entries.

`POST /deploy` — manually trigger a deploy for a named service.
```json
{"service": "myapp"}
```

**Health**

`GET /healthz` — liveness probe. Returns 200 if `ROOT_DIR` is accessible.

`GET /readyz` — readiness probe. Returns 200 if `ROOT_DIR` is accessible and `docker compose` is available.

**Flags**

All `/flags/*` endpoints require `Authorization: Bearer <API_TOKEN>`.

`GET /flags` — current flag state.

`POST /flags/pause` / `POST /flags/resume` — pause or resume webhook-triggered deployments.

`POST /flags/dry-run/enable` / `POST /flags/dry-run/disable` — log what would happen without executing.

## Observability

Structured JSON logs via `tracing`. Level controlled by `LOG_LEVEL`.

Build with `--features metrics` to expose a Prometheus `/metrics` endpoint.

Build with `--features otlp` to export traces over OTLP/gRPC (e.g. to Grafana Tempo). Requires `OTLP_ENDPOINT`.

Both features can be combined: `--features metrics,otlp`.

## Building

```bash
cargo build --release
```

Static musl binary for Linux:
```bash
cargo build --release --target x86_64-unknown-linux-musl
```

Pre-built binaries and a multi-arch Docker image are published on each tagged release.
