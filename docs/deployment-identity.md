# Relay deployment identity

Canonical relay images from `ghcr.io/block/buzz` carry two signed
attestations:

- SLSA build provenance maps the immutable image digest to the source commit
  and Docker workflow run.
- The Buzz deployment-eligibility predicate records the successful same-SHA
  CI run and the exact Buzz Helm chart version from that source commit.

The Docker workflow creates tagged multi-architecture manifests only after the
same full source SHA has a successful `CI` push run on `main` or `release`.
Architecture-specific build manifests may exist without tags while CI is
running or after it fails; they do not receive the deployment-eligibility
predicate and are not promotion inputs.

Verify a canonical eligible digest with:

```bash
gh attestation verify \
  oci://ghcr.io/block/buzz@sha256:<digest> \
  --repo block/buzz \
  --signer-workflow block/buzz/.github/workflows/docker.yml \
  --predicate-type https://buzz.block.xyz/attestations/deployment-eligibility/v1 \
  --source-ref refs/heads/main
```

The predicate's `helm_chart.compatible_version` is image-to-chart metadata. It
does not describe database schema compatibility and does not relax Buzz's rule
that migrations remain backwards compatible.

The manual pre-merge workflow publishes only to
`ghcr.io/block/buzz-staging-dev`. Those preview images are intentionally
ineligible: they use a different package, may name non-main source, and do not
receive the canonical deployment-eligibility predicate.

## Runtime inspection

The relay health listener exposes intrinsic build identity at `/_status`:

```json
{
  "service": "buzz-relay",
  "version": "0.2.1",
  "uptime_seconds": 123,
  "build": {
    "source_sha": "<40-character-source-sha>",
    "id": "github-actions:<run-id>:<attempt>",
    "url": "https://github.com/block/buzz/actions/runs/<run-id>/attempts/<attempt>"
  }
}
```

Non-CI builds report stable `unknown` or `local` fallback values instead of
claiming provenance they do not have.

## Helm digest pinning

Buzz chart `0.1.8` and newer accept an immutable image digest:

```yaml
image:
  repository: ghcr.io/block/buzz
  digest: sha256:<64-lowercase-hex-characters>
```

When `image.digest` is set, the chart renders `repository@digest` and ignores
`image.tag`. Existing tag-only values remain backwards compatible.
