# Buzz CI MinIO

`ghcr.io/block/buzz-minio:latest` contains MinIO and `mc` for the disposable
Linux AMD64 CI runners. `docker-compose.ci.yml` selects it for both services;
development and deployment defaults stay in `docker-compose.yml`.

The **MinIO image** workflow builds only when its inputs change, or on a manual
dispatch. Pull requests build and smoke-test without publishing. On `main`, a
successful smoke test publishes the same image as `sha-<full commit>` and
`latest`. Ordinary CI only pulls it; there is no build fallback or dependency
on the publisher. `latest` deliberately floats, and Compose always pulls it.
Docker's pull output records the resolved digest for each run.

The Dockerfile uses the same upstream releases as the development services,
with checksummed official GitHub release binaries and a digest-pinned Alpine
base. MinIO and `mc` are AGPL-3.0; their corresponding source is available at
the [MinIO release](https://github.com/minio/minio/tree/RELEASE.2025-09-07T16-13-09Z)
and [mc release](https://github.com/minio/mc/tree/RELEASE.2025-08-13T08-35-41Z).

## First publication

The image must exist and be publicly pullable before the CI switch can pass.
For initial rollout, land the publisher files (`.github/ci/minio/`,
`.github/workflows/minio-image.yml`, and `docker-compose.ci.yml`) first, leaving
the CI consumers unchanged. After the publisher succeeds, an org/package admin
must make **buzz-minio** public in its GitHub package settings (new GHCR
packages default to private, even for public repositories). Check an anonymous
pull of `ghcr.io/block/buzz-minio:latest`, then land the consumer changes and
rerun affected CI. Subsequent publications preserve package visibility.

## Updating or rebuilding

Update the release URLs/checksums or base digest in the Dockerfile and open a
PR. Merging triggers publication. To rebuild the existing recipe (for example,
to pick up Alpine package updates), dispatch **MinIO image** from `main`.
Other refs can build and test but cannot publish `latest`.

To reproduce a CI run with a recorded version, set `MINIO_CI_IMAGE` to
`ghcr.io/block/buzz-minio:sha-<full commit>` or its digest while using
`COMPOSE_FILE=docker-compose.yml:docker-compose.ci.yml`.
