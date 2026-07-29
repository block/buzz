# Development Setup

## Prerequisites

- **Hermit** (pinned toolchain manager) — activates automatically in the `buzz/` directory
- **Docker Desktop** — for Postgres, Redis, and MinIO containers
- Rust, Node, pnpm, Flutter — all managed by Hermit

## Quick start

```bash
# From the buzz/ directory
just dev
```

This starts the full stack via Docker Compose: Postgres 17, Redis 7, MinIO, and the relay.

## Key commands

| Command | Description |
|---|---|
| `just dev` | Start full dev environment |
| `just dev-reset` | Reset dev environment (destroy data) |
| `cargo build -p buzz-relay` | Build the relay |
| `cargo test -p buzz-conformance` | Run conformance tests |
| `just desktop` | Start the desktop client |
| `just mobile` | Start the mobile client |

## Environment

Copy `.env.example` to `.env` and configure. Key variables:
- `DATABASE_URL` — Postgres connection string
- `REDIS_URL` — Redis connection string
- `S3_ENDPOINT` — MinIO/S3 endpoint
- `RELAY_URL` — the relay's external URL

## Seeding

```bash
just seed-local-community
```

Creates a local community with sample channels and test data.

**Related:**
- [Deployment](deployment)
- [Configuration](configuration)
- [CLIReference](cli-reference)
