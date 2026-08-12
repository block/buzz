# VPS deployment path audit

Date: 2026-08-12

## Scope

This read-only audit compared the VPS deployment path at these branch tips:

- `block/buzz/main`: `4b3570671eb2786594267758af18784ac6e82972`
- `launchpad-26/buzz/main`: `f53bbd1152464ecbb1de495e2d1d959e156138f0`

The trace began at `deploy/compose/run.sh` and followed its runtime files, the
environment examples, the relay Dockerfile, and the GitHub workflow that
publishes the image consumed by Compose.

## Deployment path

```text
deploy/compose/run.sh
├── deploy/compose/.env
│   └── normally copied from deploy/compose/.env.example
├── deploy/compose/compose.yml                 always
├── deploy/compose/compose.caddy.yml           BUZZ_COMPOSE_TLS=true
│   └── deploy/compose/Caddyfile
└── deploy/compose/compose.dev.yml             BUZZ_COMPOSE_DEV=true
    └── prometheus.yml
```

The Compose stack pulls a prebuilt relay image. It does not build the root
`Dockerfile`.

The related image-publication path is:

```text
.github/workflows/docker.yml
├── Dockerfile
├── .dockerignore
└── scripts/verify-release-ref.sh
```

Because the Dockerfile contains `COPY . .`, its transitive build input is the
repository build context after `.dockerignore` exclusions.

## File-by-file comparison

Every deployment and image-publication file below had the same Git blob on both
branches:

| File | Purpose | Result |
|---|---|---|
| `deploy/compose/run.sh` | Deployment entrypoint | Identical, including executable mode |
| `deploy/compose/compose.yml` | Base production stack | Identical |
| `deploy/compose/compose.caddy.yml` | HTTPS/Caddy override | Identical |
| `deploy/compose/compose.dev.yml` | Admin/dev ports and tools | Identical |
| `deploy/compose/.env.example` | VPS environment template | Identical |
| `deploy/compose/Caddyfile` | Domain proxy to `relay:3000` | Identical |
| `deploy/compose/README.md` | VPS instructions | Identical |
| `prometheus.yml` | Dev-override Prometheus configuration | Identical |
| `.env.example` | Root application example; not used by `run.sh` | Identical |
| `Dockerfile` | Relay image build | Identical |
| `.dockerignore` | Docker build-context exclusions | Identical |
| `.github/workflows/docker.yml` | GHCR image publication | Identical |
| `scripts/verify-release-ref.sh` | Release-tag verification | Identical |

The complete `deploy/compose` tree had the same tree hash on both branches:
`17d9836bb55909fc467194a9c0c8265e80c7c33b`.

## Image-selection findings

The Launchpad deployment selected Block's image in three places:

```yaml
# deploy/compose/compose.yml
image: ${BUZZ_IMAGE:-ghcr.io/block/buzz:main}
```

```dotenv
# deploy/compose/.env.example
BUZZ_IMAGE=ghcr.io/block/buzz:main
```

The deployment README also documented `ghcr.io/block/buzz:main` as the default.
Consequently, the normal quick start explicitly configured the upstream Block
image. Removing or emptying `BUZZ_IMAGE` still caused Compose to fall back to
the same upstream image.

The supporting image workflow used:

```yaml
IMAGE_NAME: ${{ vars.GHCR_IMAGE != '' && vars.GHCR_IMAGE || 'ghcr.io/block/buzz' }}
```

At audit time, `launchpad-26/buzz` had no `GHCR_IMAGE` Actions variable and no
visible `ghcr.io/launchpad-26/buzz` package. The workflow was active but had no
recorded runs for `main`. The repository's default branch was `launchpad`, while
the inherited workflow triggered image publication only for pushes to `main`.
The Dockerfile's OCI source, URL, and documentation labels also pointed to
`block/buzz`.

Other runtime images were identical between repositories:

- `postgres:17-alpine`
- `redis:7-alpine`
- pinned MinIO server and client releases
- `caddy:2-alpine`
- dev-only `adminer:latest` and `prom/prometheus:latest`

## `run.sh` behavior

There were no differences between repositories. Both versions:

- always load `compose.yml`;
- add the Caddy and development overrides only when their switches are present
  in the invoking shell;
- invoke Docker Compose with `--env-file .env`;
- never inspect the Git remote, checkout, build, or infer an image from the
  repository;
- do not explicitly pull a newer image during `start`;
- pull and then recreate the stack during `upgrade`;
- recreate only the relay, without pulling, during `restart`; and
- bypass the shared Compose wrapper for member-management commands.

## Required mapping changes

A future deployment process must not be considered mapped to Launchpad until it
does all of the following:

1. Publish a relay image owned by Launchpad, such as
   `ghcr.io/launchpad-26/buzz`.
2. Align the publication trigger with the intended source branch and image tag.
   If `launchpad` is the source branch, it must not accidentally publish or
   consume `:main` as though it represented that branch.
3. Set the real VPS `BUZZ_IMAGE` and checked-in examples to an immutable
   Launchpad image tag or digest.
4. Remove the silent `ghcr.io/block/buzz:main` fallback, preferably requiring
   `BUZZ_IMAGE` explicitly.
5. Update OCI metadata and attestation examples to identify
   `launchpad-26/buzz`.
6. Retarget or disable the push-gateway jobs that remain hard-coded to Block's
   GHCR namespace.

## Conclusion

The two requested VPS deployment paths were identical. That identity was the
problem: Launchpad's inherited path selected `ghcr.io/block/buzz:main`, so it
could successfully start while running upstream Block code rather than code
from the Launchpad repository. The failed `launchpad/deploy` experiment was
built around that misunderstood path and must not be used as a deployment or
build method.
