# Manually publish a relay image

Use this path to build a committed source snapshot without waiting for the
normal CI qualification. It publishes a uniquely tagged image, not a release
or a deployment. Normal qualified publishing remains in `docker.yml`.

Both methods build the root Dockerfile's `runtime` target for `linux/amd64`
and `linux/arm64`, including its relay, admin, pairing binaries and web assets.
They do not build debug images or the separate push gateway.

## Laptop

Requirements: Git, Python 3.9+, a running Docker engine with Buildx, support
for both Linux architectures, and Docker credentials with permission to push
the destination repository. Log in using `docker login ghcr.io`; repository
read access alone does not grant GHCR package publishing permission. Do not
put credentials in command arguments or build args.

From your Buzz checkout, activate Hermit and select committed source:

```bash
. bin/activate-hermit
# Inspect the resolved commit and planned output; does not build or upload.
scripts/build-relay-image.sh --ref feature/my-change --dry-run

# Fetch a remote branch, tag, full SHA or refs/pull/NUMBER/head and publish.
scripts/build-relay-image.sh --ref feature/my-change --push

# Build an unpushed local commit. Uncommitted edits are not included.
scripts/build-relay-image.sh --local-ref HEAD --push

# Distributors can publish to their own registry repository.
scripts/build-relay-image.sh --local-ref HEAD --push \
  --image ghcr.io/example-org/relay --receipt /tmp/relay-build.json
```

The helper uses a clean temporary worktree and leaves your working checkout
alone. It retains the `buzz-manual` Buildx builder/cache for subsequent builds;
use `--builder NAME` to select a different builder. It does not change Docker's
selected builder. The first build may be slow; cross-architecture emulation
can take substantially longer than native CI runners. The build command has
a two-hour limit. Check Docker memory/disk capacity before a large Rust build.

A successful run prints a verified index digest and writes a JSON receipt.
An explicit `--receipt` must be a new path to avoid confusing an old success
with a failed new build. The receipt confirms registry availability and both
runtime architectures, not passing tests. An unpushed source commit cannot
be viewed or compared on GitHub until you push it to a branch.

## GitHub Actions

Once this workflow is merged to the repository's default branch:

1. Open **Actions → Manual relay image → Run workflow**.
2. Select the default branch for the workflow definition.
3. Enter the code branch, tag, full SHA or PR head ref in **source_ref**.
4. Run it and wait for **Publish and verify the combined image** to succeed.
5. Copy the image tag/digest from the summary, or download the JSON receipt.

The code selected by `source_ref` does not need to merge or have passing CI.
Hosted runners cannot access commits that exist only on your laptop; push
those to a branch first. The workflow runs only on manual dispatch, never on
pushes, PRs or a schedule. It uses native AMD64/ARM64 runners in parallel and
GitHub's package publishing token, so it does not need a laptop login.

Equivalent CLI invocation (adjust the repository for your fork):

```bash
gh workflow run docker-manual.yml --repo block/buzz --ref main \
  -f source_ref=feature/my-change
gh run list --repo block/buzz --workflow docker-manual.yml
```

The workflow definition and publishing verifier come from the workflow's
commit. Only the build source comes from `source_ref`. All architectures
receive the same resolved SHA, not the workflow definition's SHA. Forks can
configure the repository variable `GHCR_IMAGE` to their own GHCR repository.

## Outputs and failures

Tags start with `manual-` and contain the source SHA and unique build identity.
Normal branch, semver and release tags are not changed. The image labels and
compiled build identity record its source. No successful-CI or deployment
eligibility claim is issued for these builds.

If one architecture fails, the workflow does not publish a combined tag.
Per-architecture digests are retained as artifacts for seven days. A rerun
gets a separate final tag through its run attempt. Missing platforms, source
mismatches and registry errors fail verification and prevent a success receipt.
Check the failing step; do not deploy a single-architecture digest as if it
were the combined image. A failed upload can leave blobs in the registry,
but those are not a verified deployment artifact.

Import the digest into any downstream registry using your deployment
environment's normal procedure, verify availability there, and use the digest
in a separate deployment change. GHCR publication alone does not prove a
downstream cache has imported the image or that an application is healthy.
