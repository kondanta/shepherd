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
| `SERVICE_FILTER` | no | — | Comma-separated names of services this instance may deploy — these are the keys under `services:` in the compose file, not folder names; unset means all |
| `ALLOW_LATEST_IMAGES` | no | `false` | Allow services tagged `latest` to be deployed |
| `INITIAL_SYNC` | no | `true` | On startup, pull and apply all services using idempotent `docker compose up -d --no-deps`. Catches any drift that accumulated while Shepherd was down. Set to `false` to disable. |
| `SHEPHERD_SERVICE_NAME` | no | — | The compose service name Shepherd itself runs as. When set, Shepherd deploys itself last in any batch and uses a short-lived `shepherd-updater` container to perform its own update — this avoids a race where Shepherd would be killed before it could start its own replacement. Must match the service key under `services:` exactly (e.g. `shepherd`). |
| `API_TOKEN` | no | — | Bearer token required for `/flags/*` and `/sync` endpoints |
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
      SHEPHERD_SERVICE_NAME: shepherd
```

### Self-updating

When `SHEPHERD_SERVICE_NAME` is set, Shepherd handles its own container replacement differently. Updating itself through the normal `docker compose up` path would fail: Docker Compose stops the running container before starting the replacement, and since Shepherd is PID 1 in its container, it is killed before the start command can run — leaving the new container stuck in `Created` state.

Instead, Shepherd spawns a separate `shepherd-updater` container (using its own image, which includes the Docker CLI) to run `docker compose up --force-recreate` from outside its own container namespace. The updater survives Shepherd's shutdown and completes the transition.

The `shepherd-updater` container name is reserved — do not use it for any other service.

On startup, Shepherd removes any leftover `shepherd-updater` container before performing the initial sync, recovering cleanly from a previous interrupted update.

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

`POST /sync` — trigger an immediate reconciliation of all services on disk, identical to the startup sync. Returns 202. Returns 503 if deployments are paused.

**Health**

`GET /healthz` — liveness probe. Returns 200 if `ROOT_DIR` is accessible.

`GET /readyz` — readiness probe. Returns 200 if `ROOT_DIR` is accessible and `docker compose` is available.

**Flags**

All `/flags/*` and `/sync` endpoints require `Authorization: Bearer <API_TOKEN>`.

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
