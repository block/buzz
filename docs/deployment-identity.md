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

## Derived deployment labels

The storage-accounting CronJob has no runtime `/_status` surface to interrogate,
so the chart stamps its Pods with the deployed image identity instead of asking
an operator to restate it. `BUZZ_STORAGE_SNAPSHOT_CODE_SHA` (persisted as
`code_sha` on every snapshot row) and the Pod's `tags.datadoghq.com/version`
label are both derived from `image.digest` — or `image.tag`, or
`Chart.AppVersion` — so a snapshot's telemetry version and its recorded version
cannot drift apart.

The environment variable always keeps the exact revision. The label cannot: a
label value is capped at 63 bytes, must begin and end with an alphanumeric, and
may otherwise contain only `[-._a-zA-Z0-9]`, while `image.tag` accepts any OCI
tag. Because that domain is larger than the label codomain, no mapping onto it
is injective; the chart provides a deterministic, collision-resistant one. A
digest keeps the first 63 characters of its hex, a revision that is already a
valid label value is preserved byte for byte, and anything else (a leading `_`,
a byte outside the label alphabet, more than 63 bytes) is replaced by the first
63 hex characters of its SHA-256 — 252 retained bits, the same margin as the
digest case, and no readable prefix.

Exactly 63 lowercase hex characters is reserved for those hashed and digest
forms: a tag of that shape is hashed instead of preserved, so a rendered label
cannot be copied into `image.tag` to make two revisions report one version.
Shorter hex tags and 40-character git SHAs pass through unchanged.

Any `tags.datadoghq.com/version` supplied through `storageAccounting.podLabels`
is ignored; see the chart README's "Storage accounting worker" section.
