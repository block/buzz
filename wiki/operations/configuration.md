# Configuration

Buzz is configured primarily through environment variables.

## Core

| Variable | Description |
|---|---|
| `DATABASE_URL` | Postgres connection string |
| `REDIS_URL` | Redis connection string |
| `RELAY_URL` | Public URL of the relay |
| `RELAY_PORT` | Listen port (default: 8080) |

## Media (S3/MinIO)

| Variable | Description |
|---|---|
| `S3_ENDPOINT` | S3-compatible endpoint URL |
| `S3_BUCKET` | Bucket name for media storage |
| `S3_REGION` | AWS region (default: us-east-1) |
| `S3_ACCESS_KEY` | Access key |
| `S3_SECRET_KEY` | Secret key |

## Auth

| Variable | Description |
|---|---|
| `RATE_LIMIT_MESSAGES` | Messages per minute per connection |
| `RATE_LIMIT_CONNECTIONS` | Max connections per IP |

See `.env.example` for the full list.

**Related:**
- [DevelopmentSetup](development-setup)
- [Deployment](deployment)
