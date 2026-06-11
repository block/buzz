# Buzz Helm Chart

[Buzz](https://github.com/block/buzz) is a Nostr-based messaging platform for human–agent collaboration: a single relay binary serving WebSocket + REST + web UI, backed by PostgreSQL, Redis, Typesense, and S3-compatible object storage.

This chart has two operating profiles selected by values:

| Profile | When | What you get |
|---|---|---|
| **Production** (default) | Self-hosted multi-tenant, regulated, or GitOps-managed | External managed Postgres/Redis/Typesense/S3, `secrets.existingSecret:`, no chart-side autogen, HA-capable (`replicaCount ≥ 2`) |
| **Quickstart** (`--set quickstart=true`) | Eval, single-node, one-off demo | In-cluster Postgres + Redis subcharts ([CloudPirates](https://github.com/cloudpirates)), chart auto-generates relay secrets, single replica |

## Quickstart (eval only)

```sh
helm install buzz oci://ghcr.io/block/buzz/charts/buzz --version 0.1.0 \
  --create-namespace --namespace buzz \
  --set quickstart=true \
  --set postgresql.enabled=true \
  --set redis.enabled=true \
  --set relayUrl=wss://buzz.example.com \
  --set ownerPubkey=<64-char-hex-pubkey> \
  --set typesense.url=http://typesense.buzz.svc.cluster.local:8108 \
  --set typesense.apiKey=<typesense-key>
```

Quickstart still requires an externally managed Typesense in v1; bring up a minimal Typesense Pod/StatefulSet in your namespace, or set `typesense.url` and `typesense.apiKey` to a hosted instance. See the open question in `OPEN_QUESTIONS` at the bottom of this README.

## Production (GitOps)

The chart is designed for ArgoCD and Flux. Both render charts with `helm template`, in which mode Helm's `lookup` function returns empty — any chart-side `randAlphaNum` call would regenerate secrets on every sync. The chart-managed Secret path is **only** safe for `helm install` / `helm upgrade`.

Production deploys MUST use `secrets.existingSecret:`. The Secret is consumed for any keys present and ignored for keys missing — extras are harmless.

See:

- [`examples/argocd-app.yaml`](examples/argocd-app.yaml) — ArgoCD Application
- [`examples/flux-helmrelease.yaml`](examples/flux-helmrelease.yaml) — Flux HelmRelease v2
- [`examples/secret-sample.yaml`](examples/secret-sample.yaml) — Secret schema

## Required inputs

| Key | What | When required |
|---|---|---|
| `relayUrl` | Public `wss://` URL clients connect to | Always |
| `ownerPubkey` | 64-char lowercase hex Nostr pubkey of the relay operator | When `relay.requireRelayMembership=true` (default) |
| `secrets.existingSecret` | Name of pre-created Secret | Production / GitOps |
| `externalPostgresql.url` / `externalRedis.url` / `typesense.url` | External service URLs | When the matching subchart is disabled (default) |

The chart fails at `helm install` / `helm template` time with a clear message if any of these are missing or malformed (see `templates/_validate.tpl`).

## HA (production)

`replicaCount > 1` hard-requires both:

- Redis (`redis.enabled=true`, `externalRedis.url`, or `REDIS_URL` in `existingSecret`) — for `buzz-pubsub` fan-out
- ReadWriteMany git PVC — `persistence.git.accessMode: ReadWriteMany` with a RWX storage class (e.g. `efs-sc` on AWS, `azurefile-csi` on Azure)

The chart **template-fails** if either invariant is broken. No silent degradation.

## Upgrades

Schema migrations are embedded in the relay binary via `sqlx::migrate!` and run at startup, gated by `BUZZ_AUTO_MIGRATE` (default `true`). Multiple replicas race-safely behind a Postgres advisory lock. `helm upgrade` is the entire upgrade procedure.

If you prefer decoupling migrations from serving, set `migrate.autoMigrate=false`. **In that mode the chart does not run migrations for you** — you own running `buzz-admin migrate` (separate Pod / one-shot Job) against the database before every `helm install` / `helm upgrade`. Readiness probes only verify DB connectivity, not schema freshness, so a pod will appear healthy against an unmigrated schema and fail under load. A pre-upgrade Helm Job for this is on the chart roadmap; the values knob `migrate.preUpgradeJob.enabled` is reserved.

## Backups

Save these. Losing any of them is data loss. See NOTES.txt printed by `helm install` for the live list:

1. `BUZZ_RELAY_PRIVATE_KEY` — relay identity. Rotating it = new identity (federation peers will not recognize the relay).
2. PostgreSQL database — the canonical event store.
3. S3 bucket — media blobs (chart default bucket: `buzz-media`).
4. Git PVC — repo on-disk state served by the relay's git endpoint.
5. Owner private key — held by the operator, not by this chart. Restore by re-installing with the same `ownerPubkey`.

## Honest limitations (v1)

- **Typesense has no in-chart subchart.** Bring your own Typesense; the chart wires it via `typesense.url` + `typesense.apiKey` (or `TYPESENSE_URL` / `TYPESENSE_API_KEY` in `existingSecret`). The roadmap depends on either an upstream community chart hitting our quality bar or a minimal in-chart StatefulSet behind a quickstart flag.
- **Minimal-mode is not yet supported.** The relay's `BUZZ_PUBSUB=local` / `BUZZ_SEARCH=pg` / filesystem media paths are upstream work in progress. Until then, "quickstart" still needs Typesense.
- **OCI publish to GHCR + cosign signing** is a follow-up PR. For now, install the chart from source: `helm install buzz ./deploy/charts/buzz` after cloning the repo.

## Development

```sh
# Render every fixture
for f in ci/*-values.yaml tests/fixtures/*-values.yaml; do
  helm template buzz . -f "$f" >/dev/null && echo "ok: $f"
done

# Unit tests
helm plugin install https://github.com/helm-unittest/helm-unittest
helm unittest .

# Lint
helm dependency build .
ct lint --config ../../../ct.yaml --charts .
```
