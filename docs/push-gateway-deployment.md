# Buzz Push Gateway deployment

`buzz-push-gateway` is the standalone public APNs last hop intended for `push.buzz.xyz`. Build it with `Dockerfile.push-gateway`; do not run it in the relay image or give relays APNs credentials.

## Network and health

- Public listener: `BUZZ_PUSH_BIND_ADDR` (default `0.0.0.0:8080`). Route `https://push.buzz.xyz` to this port.
- Private health listener: `BUZZ_PUSH_HEALTH_ADDR` (default `0.0.0.0:8081`). Probe `/_liveness` and `/_readiness`; do not expose this port publicly. The chart has no pod-ingress allowance for 8081; Kubernetes node/kubelet-origin probe traffic is exempt from NetworkPolicy. Add a narrowly selected monitoring source only if the target CNI requires pod-origin health scraping.
- Readiness fails when PostgreSQL authority is unavailable. Graceful shutdown stops accepting new requests before draining in-flight APNs calls.

## Required configuration

| Variable | Purpose |
|---|---|
| `DATABASE_URL` | PostgreSQL authority/admission store. Runtime credentials need DML on the six gateway tables, not DDL. |
| `BUZZ_PUSH_PUBLIC_DELIVERY_URL` | Exact externally signed URL, normally `https://push.buzz.xyz/v1/deliveries/apns`. |
| `BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS` | Maximum delegation capability lifetime (`1..=31536000`). |
| `BUZZ_PUSH_MAX_INSTALLATION_LIFETIME_SECONDS` | Maximum encrypted-token installation lifetime (default 90 days, max one year). Clients must renew before expiry. |
| `BUZZ_PUSH_APP_ATTEST_ROOT_CERT_PATH` | Read-only mounted Apple App Attest root certificate PEM. |
| `BUZZ_PUSH_DOGFOOD_APP_ATTEST_APP_ID` | Exact server-owned Apple App Attest application identifier (`TEAMID.bundle-id`). |
| `BUZZ_PUSH_DOGFOOD_APNS_TOPIC` | Server-owned APNs topic. Never accepted from a client. |
| `BUZZ_PUSH_DOGFOOD_APNS_ENVIRONMENT` | `production` or `sandbox`, selected by deployment configuration. |
| `BUZZ_PUSH_DOGFOOD_APNS_CERT_PATH` | Read-only certificate/private-key PEM. |
| `BUZZ_PUSH_GRANT_KEYS` | Capability AEAD keyring, `id:base64-32-bytes[,predecessor...]`; current key first. |
| `BUZZ_PUSH_TOKEN_KEYS` | Independent token-custody AEAD keyring in the same format. Never reuse grant keys. |

The canonical `push.buzz.xyz` MVP serves the dogfood application identity
(`xyz.block.buzz.dogfood.mobile`). App Attest must cryptographically validate
the configured application ID before enrollment. Assertions and delivery use
the server-owned APNs topic, certificate-backed connection pool, and
environment. No client request or relay grant can supply or override an APNs
topic.

This MVP has exactly one compiled-in application profile,
`buzz-ios-dogfood`. The chart value
`profiles.dogfood.appAttestAppId` is rendered as
`BUZZ_PUSH_DOGFOOD_APP_ATTEST_APP_ID`; the gateway rejects startup when it is
missing or empty. The exact `TEAMID.bundle-id` is environment-owned,
non-secret deployment configuration. The chart's production values file leaves
it empty deliberately so a production renderer must supply it from the GitOps
environment rather than baking a Block team identifier into this repository.
Supporting another application identity requires an explicit code, schema,
chart, credential, and deployment change; this gateway does not currently
select among multiple application profiles.

Optional endpoint quota policy variables are `BUZZ_PUSH_ENDPOINT_QUOTA_WINDOW_SECONDS` (default `10`, max `86400`) and `BUZZ_PUSH_ENDPOINT_QUOTA_MAX_DELIVERIES` (default `10`, max `10000`). These are Buzz policy hypotheses, not Apple-published limits; tune under load while retaining a hard ceiling.

## Secret and key rotation rules

Mount the App Attest root read-only and startup will reject any byte mismatch. The sole accepted artifact is Apple’s **Apple App Attestation Root CA** from `https://www.apple.com/certificateauthority/Apple_App_Attestation_Root_CA.pem`: certificate SHA-256 fingerprint `1C:B9:82:3B:A2:8B:A6:AD:2D:33:A0:06:94:1D:E2:AE:4F:51:3E:F1:D4:E8:31:B9:F7:E0:FA:7B:62:42:C9:32`; exact PEM-file SHA-256 `c778d09ac341f7fd9f8f3b19e2b815af6aed4ad4490e1e92c05cb355212a5013`. Treat an Apple root rotation as a reviewed code/config rollout, not an unpinned mount replacement. Mount the APNs certificate identity and both AEAD keyrings from a secret manager; never place values in an image, manifest, log, or metrics label. Keep the current AEAD key first and retain decrypt-only predecessors until every capability/token encrypted under them has expired or been re-encrypted. Grant and token key ids and bytes must be distinct. Rotation is an operator rollout: add the new current key while retaining predecessors, deploy, wait through the retention window, then remove the old key.

The gateway stores APNs tokens encrypted in PostgreSQL. Database backups therefore contain ciphertext plus authority metadata and must receive the same access controls and retention treatment as the service secrets.

## PostgreSQL and replicas

The gateway's dedicated pool does not consume the relay-oriented `BUZZ_DB_LOCK_TIMEOUT_MS`, `BUZZ_DB_IDLE_TXN_TIMEOUT_MS`, or `BUZZ_DB_STATEMENT_TIMEOUT_MS` settings. Its session-timeout policy remains separate from the `buzz-db` writer policy and must be designed and rolled out independently.

All replicas must share one PostgreSQL database. Delivery authority, replay admission, and endpoint quota reservation are transactional there, so replica count does not multiply the abuse ceiling. The gateway owns a scoped migration history under `crates/buzz-push-gateway/migrations`; it creates only the six `push_gateway_*` authority tables plus SQLx's migration-history table and never runs relay migrations.

The Helm chart runs a single pre-install/pre-upgrade migration Job using `migration.existingSecret`; that secret contains a DDL-capable `DATABASE_URL`. The URL MUST name a dedicated gateway database, not the relay database: SQLx stores its `_sqlx_migrations` history in `public`, so sharing a database would collide with another application's migration history. `migration.runtimeDatabaseRole` names an existing LOGIN role (the default is `buzz_push_gateway_runtime`) used by runtime `DATABASE_URL`. After scoped migrations, the Job revokes database `CREATE` from that role and schema `CREATE` from both `PUBLIC` and the role, then grants only database `CONNECT`, schema `USAGE`, and `SELECT, INSERT, UPDATE, DELETE` on the six gateway tables. The migration role must own the database/schema objects or otherwise be allowed to issue those grants; it is never provided to runtime replicas. Readiness rejects an empty/partial schema, missing DML, or a runtime role that retains database/schema `CREATE`. Helm waits for the migration hook before updating replicas, so rolling deployments never race unconditional startup migration. Readiness must be removed from load-balancer service endpoints before terminating a pod.

The service reaps expired challenges and replay rows, idle quota rows, expired/revoked delegations, and retention-eligible installations (including their encrypted token ciphertext) at startup and every five minutes. Monitor reaper failures and table growth; retention does not depend on process restarts.

## Metrics and alerting

The gateway serves Prometheus metrics at `GET /metrics` on the **private health listener** (`BUZZ_PUSH_HEALTH_ADDR`, default `0.0.0.0:8081`) — the same port as the probes, never on the public `8080`. All series are sanitized and bounded-cardinality: label values are drawn only from closed sets (the five APNs outcome classes, the fixed admission results, the static error codes already returned to callers, and the readiness causes). No endpoint, device token, relay pubkey, request id, or any request-scoped identifier is ever used as a label.

| Metric | Type | Labels | Meaning |
|---|---|---|---|
| `push_gateway_apns_send_attempts_total` | counter | none | Entries into the concrete APNs HTTP send seam. Compare with terminal outcomes to detect work that never reached transport. |
| `push_gateway_apns_deliveries_total` | counter | `outcome` = `accepted` \| `invalid_endpoint` \| `retry` \| `configuration_fault` \| `permanent_request_fault` | Terminal APNs send outcomes. |
| `push_gateway_apns_delivery_seconds` | histogram | — | APNs send round-trip latency (seconds). |
| `push_gateway_admissions_total` | counter | `result` = `admitted` \| `rejected` \| `unavailable` | Outcome at the `authorize_delivery` replay/quota fence. |
| `push_gateway_delivery_errors_total` | counter | `class` (static) | Selected delivery-handler exit classes only (see note). |
| `push_gateway_reaper_failures_total` | counter | — | Retention reaper sweep failures. |
| `push_gateway_readiness_failures_total` | counter | `cause` = `not_accepting` \| `authority` | Readiness probe failures by cause. |

`push_gateway_delivery_errors_total` is intentionally **narrow**: it counts only selected exit classes of the `/v1/deliveries/apns` handler. `class` ∈ `invalid_grant` (grant rejected at the admission seam, before a permit is issued), `rate_limited`, `temporarily_unavailable` (authority unavailable at the admission seam), `profile_mismatch`, `profile_disabled`, `token_custody` (endpoint-token open failure), `finish_failed` (detached disposition/join failure returned as 503). Request/auth/attestation/grant validation on the enrollment, delegation, rotation, and revocation handlers is **not** counted by this metric; it is a delivery-hot-path signal, not a total error rate across the API.

Scraping is **opt-in** and off by default, so the default chart render is unchanged and `8081` keeps no pod ingress. For prometheus-operator, set `podMonitor.enabled=true` and `networkPolicy.monitoring.enabled=true` with `networkPolicy.monitoring.namespaceSelector` / `podSelector` naming your scraper. For Datadog Autodiscovery, leave `podMonitor.enabled=false`, supply the OpenMetrics check through `podAnnotations`, and enable the same narrowly selected NetworkPolicy ingress:

```yaml
podAnnotations:
  ad.datadoghq.com/gateway.checks: |
    {
      "openmetrics": {
        "init_config": {},
        "instances": [{
          "openmetrics_endpoint": "http://%%host%%:8081/metrics",
          "service": "buzz-push-gateway",
          "namespace": "block.buzz_push_gateway",
          "metrics": ["push_gateway_.*"],
          "histogram_buckets_as_distributions": true,
          "send_distribution_buckets": true,
          "send_monotonic_counter": true,
          "collect_counters_with_distributions": true
        }]
      }
    }
networkPolicy:
  monitoring:
    enabled: true
    namespaceSelector: # replace with the Datadog Agent namespace labels
      kubernetes.io/metadata.name: datadog
    podSelector: # replace with the Datadog Agent pod labels
      app.kubernetes.io/name: datadog-agent
```

Both modes add one `8081` ingress rule scoped to the configured source, never a blanket allowance. Node/kubelet-origin probe traffic remains exempt from NetworkPolicy regardless. Do not enable `PodMonitor` in clusters without its CRD.

Alerting rules ship as an opt-in prometheus-operator `PrometheusRule` (`prometheusRule.enabled=true`). Thresholds and operator actions:

| Alert | Fires when | Severity | Action |
|---|---|---|---|
| `PushGatewayConfigurationFault` | any `configuration_fault` outcomes for 10m | critical | The APNs certificate/topic/environment is unhealthy. Check `BUZZ_PUSH_DOGFOOD_APNS_*` configuration. No endpoints are being invalidated. |
| `PushGatewayAdmissionUnavailable` | any admission `unavailable` for 5m | critical | PostgreSQL authority store is unreachable. Check DB connectivity and the pod's `postgresEgressCidrs` NetworkPolicy. |
| `PushGatewayReadinessAuthorityFailing` | readiness `authority` failures for 5m | warning | Replicas are being pulled from the Service on DB check failure. Fix DB health before capacity drops below the PodDisruptionBudget. |
| `PushGatewayReaperFailing` | reaper failed ≥2 times within 30m (runs every 5m) | warning | Expired reservations aren't being swept, growing the bounded-until-expiry window. Check DB write availability. |
| `PushGatewayHighApnsRetryRate` | retryable fraction > `prometheusRule.apnsRetryRatioThreshold` (default `0.25`) over a 10m window, above `apnsRetryMinSamples` (default `20`) attempts, held true for 15m | warning | APNs is throttling or degraded (429/500/503). Deliveries are delayed, not lost. |

## Relay configuration

Relay push is an explicit deployment opt-in through `BUZZ_PUSH_ENABLED=true`;
the established strict boolean parser rejects unknown values and the default is
false. When enabled, an absent `BUZZ_PUSH_GATEWAY_DELIVERY_URL` selects the exact
canonical URL `https://push.buzz.xyz/v1/deliveries/apns`; operators can provide
another exact HTTPS `/v1/deliveries/apns` URL as an advanced override. An
explicitly empty URL while enabled is a startup error. Only an enabled relay
advertises its host-scoped NIP-PL descriptor, accepts leases, and starts the
matcher and delivery worker. Relays retain lease matching, authorization, durable
jobs/retries, and generation checks; they receive only opaque capabilities and
never APNs tokens or provider credentials.

An enabled relay exports the following bounded-cardinality series on its
existing Prometheus endpoint. None carries a community, account, relay key,
installation, event, or request identifier as a label.

| Metric | Type | Labels | Meaning |
|---|---|---|---|
| `buzz_push_enabled` | gauge | — | `1` only when the deployment opt-in is active. |
| `buzz_push_match_jobs_total` | counter | `result` = `matched` \| `unmatched` \| `error` \| `context_error` | Accepted message events evaluated by the matcher. |
| `buzz_push_match_queue_seconds` | histogram | — | Relay receipt to matcher evaluation latency. |
| `buzz_push_wakes_total` | counter | `result` = `enqueued` \| `duplicate` \| `inactive_lease` | Durable wake-enqueue outcomes. |
| `buzz_push_wake_enqueue_errors_total` | counter | — | Set-wise outbox transactions that failed. |
| `buzz_push_wake_queue_seconds` | histogram | — | First-attempt outbox enqueue-to-worker latency. |
| `buzz_push_gateway_requests_total` | counter | — | Relay requests that reached the gateway transport seam. |
| `buzz_push_gateway_request_seconds` | histogram | — | Relay-observed gateway request latency. |
| `buzz_push_deliveries_total` | counter | `outcome` (static closed set) | Accepted, retried, suppressed, exhausted, invalid, or failed relay delivery outcomes. |

## Relay integration status

The operational relay integration is complete: per-origin event matching with
read-authorization checks, durable enqueue, send-time revalidation, and NIP-98
delivery run only when `BUZZ_PUSH_ENABLED=true`. End-to-end use still requires
the client App Attest enrollment/delegation flow to place a gateway-issued opaque
capability—not a raw APNs token—into the encrypted relay lease.

## Internal dogfood evaluation and rollback

The MVP is ready to enable only when the canonical gateway's sole dogfood
profile is configured with its server-owned App Attest app ID, APNs topic,
production certificate identity, and production APNs environment, and only the
selected internal relay deployments set `BUZZ_PUSH_ENABLED=true`. Every iOS
artifact contains the native push bridge and Notification Service Extension,
but the client remains inactive until its current authenticated relay
advertises a fully valid NIP-11 `nip-pl` descriptor. There is no App Store
gateway profile in this MVP.

Physical-device validation must use an application whose App Attest identity
and APNs topic match the configured dogfood profile. The current gateway cannot
enroll `xyz.block.buzz.mobile` or another bundle identifier merely by changing
deployment values: adding another identity requires the explicit multi-profile
work described above.

Dogfood end-to-end release validation starts after this feature reaches `main`:
publish the next immutable `mobile-vX.Y.Z-rc.N` candidate from the exact current
`origin/main` commit, build that tag through the normal Block release pipeline,
and wait for the signed `xyz.block.buzz.dogfood.mobile` artifact to appear in
Mobile Releases/Comp Portal before installing it on a physical device. Verify
APNs delivery, fetched and signature-verified notification content, and
exact-message tap routing against the canonical gateway and a push-enabled
internal relay before widening the internal evaluation.

Before that first candidate, the private dogfood builder's manual signing and
export configuration must map separate distribution
profiles for both `xyz.block.buzz.dogfood.mobile` and
`xyz.block.buzz.dogfood.mobile.NotificationService`; an app-only profile does
not provision the extension. App Store rollout remains off through relay and
gateway deployment configuration until separately approved.
Before enabling rich message presentation, enable Apple's Communication
Notifications capability on the parent dogfood App ID and regenerate its app
provisioning profile. The extension profile does not need that capability.
Apply the same parent-App-ID prerequisite to the eventual App Store rollout;
updating Block Apple portal records is a separately authorized release step.

For each evaluation cohort, measure relay receipt-to-match, wake queue, relay-to-
gateway, and gateway-to-APNs latencies from the histograms above. Track the
ratio of accepted or replay-terminal relay outcomes to newly enqueued wakes,
gateway APNs accepted/retry/invalid/configuration outcomes, retry exhaustion,
and NSE resolution fallback. APNs acceptance cannot prove device presentation:
record a small manual physical-device sample with event-created, banner-visible,
and notification-tap timestamps, and verify that the visible title/body came
from fetched, signature-verified relay content and that the tap opened the exact
triggering message. Keep fallback-to-channel and placeholder/failure cases as
explicit counts in the manual sample until privacy-preserving client telemetry
is designed.

Rollback does not require deleting credentials or mutating existing leases.
Set `BUZZ_PUSH_ENABLED=false` on the enabled relays to stop advertisement, lease
acceptance, matching, workers, and new gateway traffic. If the gateway itself
is unhealthy, disable the gateway deployment only after relay delivery is off.
Existing leases and gateway authorities then expire naturally. Adding an App
Store application profile is outside this internal evaluation.

## Helm production inputs

The chart defaults to the `main` image tag because `.github/workflows/docker.yml` publishes it from the push-gateway lane. For a production rollout, open that workflow run's **Publish public push gateway image** job summary and copy its `sha256:...` digest. Verify the published subject and provenance before injecting it:

```bash
gh attestation verify \
  oci://ghcr.io/block/buzz-push-gateway@sha256:<64-lowercase-hex> \
  --repo block/buzz \
  --signer-workflow block/buzz/.github/workflows/docker.yml \
  --source-digest <40-lowercase-hex-source-commit>
```

Only after that command succeeds, inject the exact digest as `image.digest` in
the environment's GitOps values; the chart then renders
`ghcr.io/block/buzz-push-gateway@sha256:...` and ignores the mutable tag.
`values-production.yaml` remains an intentionally invalid production-input
contract: deployment CI must inject the verified image digest, the provisioned
dogfood Apple application identifier, and the actual PostgreSQL network. In an
environment with an existing ingress or service mesh route, keep
`httpRoute.enabled=false`. If this chart owns a Gateway API route, enable it and
inject an environment-owned `parentRef`; schema validation rejects an enabled
route with no parent. The render guard proves both rejection of missing required
inputs and fully injected renders.

Network policy keeps APNs HTTPS and PostgreSQL egress in separate CIDR lists. APNs currently requires broad TCP/443 reachability; `networkPolicy.postgresEgressCidrs` must be narrowed to the production database network, and the DNS namespace/pod selectors must match the cluster DNS deployment. The sample private CIDR is not a claim about the production topology.

Kubernetes does not restart pods when referenced Secret bytes change. AEAD or APNs certificate rotation therefore requires an explicit rolling restart after the secret manager update (for example, `kubectl rollout restart deployment/<release>-buzz-push-gateway`) and readiness verification before removing predecessor keys. Service-account token automount is disabled.

## Gateway chart release

The gateway chart has a collision-free release lane separate from the main
`buzz` chart. To publish chart version `X.Y.Z`, update `version` in
`deploy/charts/buzz-push-gateway/Chart.yaml` and keep `appVersion` equal to the
gateway binary's workspace package version. Validate the chart, then open a
same-repository PR whose branch is exactly `push-chart-release/X.Y.Z`:

```bash
deploy/charts/buzz-push-gateway/tests/render.sh
git switch -c push-chart-release/X.Y.Z
git add deploy/charts/buzz-push-gateway/Chart.yaml
git commit -m "release: push gateway chart X.Y.Z"
git push -u origin push-chart-release/X.Y.Z
```

When that PR merges, `.github/workflows/auto-tag-on-release-pr-merge.yml`
creates `push-chart-vX.Y.Z` and dispatches
`.github/workflows/push-gateway-helm-chart.yml` with that immutable tag and bare
version. The publisher verifies the checked-out commit is the tag target and the
chart version equals `X.Y.Z` before pushing
`oci://ghcr.io/block/buzz/charts/buzz-push-gateway`. A manually pushed
`push-chart-vX.Y.Z` tag is the documented rescue path and runs the same checks.
After the publisher succeeds, inspect and fetch the published chart version
before use:

```bash
helm show chart oci://ghcr.io/block/buzz/charts/buzz-push-gateway --version X.Y.Z
helm pull oci://ghcr.io/block/buzz/charts/buzz-push-gateway --version X.Y.Z
```
