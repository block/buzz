# Deployment

## Docker Compose (Production)

A production-ready Docker Compose bundle is available in `deploy/compose/`. It includes:
- `buzz-relay` (with all service crates compiled in)
- PostgreSQL 17
- Redis 7
- MinIO (S3-compatible storage)
- Prometheus (metrics)

## Helm (Kubernetes)

Helm charts are in `deploy/charts/`. Suitable for production multi-tenant deployments.

## Requirements

- PostgreSQL 17
- Redis 7
- S3-compatible object storage (MinIO, AWS S3, etc.)
- TLS termination (reverse proxy with Let's Encrypt)

## Configuration

All configuration is via environment variables. See [Configuration](configuration) for the full list.

**Related:**
- [DevelopmentSetup](development-setup)
- [Configuration](configuration)
- [MultiTenancy](../concepts/multi-tenancy)
