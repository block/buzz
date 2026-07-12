# Buzz Push Gateway deployment

`buzz-push-gateway` is the standalone public APNs last hop intended for `push.buzz.xyz`. Build it with `Dockerfile.push-gateway`; do not run it in the relay image or give relays APNs credentials.

## Network and health

- Public listener: `BUZZ_PUSH_BIND_ADDR` (default `0.0.0.0:8080`). Route `https://push.buzz.xyz` to this port.
- Private health listener: `BUZZ_PUSH_HEALTH_ADDR` (default `0.0.0.0:8081`). Probe `/_liveness` and `/_readiness`; do not expose this port publicly. The chart has no pod-ingress allowance for 8081; Kubernetes node/kubelet-origin probe traffic is exempt from NetworkPolicy. Add a narrowly selected monitoring source only if the target CNI requires pod-origin health scraping.
- Readiness fails when PostgreSQL authority is unavailable. Graceful shutdown stops accepting new requests before draining in-flight APNs calls.

## Required configuration

| Variable | Purpose |
|---|---|
| `DATABASE_URL` | PostgreSQL authority/admission store; startup applies embedded migrations. |
| `BUZZ_PUSH_PUBLIC_DELIVERY_URL` | Exact externally signed URL, normally `https://push.buzz.xyz/v1/deliveries/apns`. |
| `BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS` | Maximum delegation capability lifetime (`1..=31536000`). |
| `BUZZ_PUSH_MAX_INSTALLATION_LIFETIME_SECONDS` | Maximum encrypted-token installation lifetime (default 90 days, max one year). Clients must renew before expiry. |
| `BUZZ_PUSH_ENABLED_PROFILES` | Comma-separated `buzz-ios-production` and/or `buzz-ios-sandbox`. |
| `BUZZ_PUSH_APP_ATTEST_APP_ID` | Exact Apple App Attest application identifier (`TEAMID.bundle-id`). |
| `BUZZ_PUSH_APP_ATTEST_ROOT_CERT_PATH` | Read-only mounted Apple App Attest root certificate PEM. |
| `BUZZ_PUSH_APNS_KEY_PATH` | Read-only mounted Apple APNs `.p8` provider key. |
| `BUZZ_PUSH_APNS_KEY_ID` | APNs provider key id. |
| `BUZZ_PUSH_APNS_TEAM_ID` | Apple developer team id. |
| `BUZZ_PUSH_APNS_TOPIC` | Buzz iOS bundle id. |
| `BUZZ_PUSH_GRANT_KEYS` | Capability AEAD keyring, `id:base64-32-bytes[,predecessor...]`; current key first. |
| `BUZZ_PUSH_TOKEN_KEYS` | Independent token-custody AEAD keyring in the same format. Never reuse grant keys. |

Optional endpoint quota policy variables are `BUZZ_PUSH_ENDPOINT_QUOTA_WINDOW_SECONDS` (default `10`, max `86400`) and `BUZZ_PUSH_ENDPOINT_QUOTA_MAX_DELIVERIES` (default `10`, max `10000`). These are Buzz policy hypotheses, not Apple-published limits; tune under load while retaining a hard ceiling.

## Secret and key rotation rules

Mount the App Attest root read-only and startup will reject any byte mismatch. The sole accepted artifact is Apple’s **Apple App Attestation Root CA** from `https://www.apple.com/certificateauthority/Apple_App_Attestation_Root_CA.pem`: certificate SHA-256 fingerprint `1C:B9:82:3B:A2:8B:A6:AD:2D:33:A0:06:94:1D:E2:AE:4F:51:3E:F1:D4:E8:31:B9:F7:E0:FA:7B:62:42:C9:32`; exact PEM-file SHA-256 `c778d09ac341f7fd9f8f3b19e2b815af6aed4ad4490e1e92c05cb355212a5013`. Treat an Apple root rotation as a reviewed code/config rollout, not an unpinned mount replacement. Mount the APNs key and both AEAD keyrings from a secret manager; never place values in an image, manifest, log, or metrics label. Keep the current AEAD key first and retain decrypt-only predecessors until every capability/token encrypted under them has expired or been re-encrypted. Grant and token key ids and bytes must be distinct. Rotation is an operator rollout: add the new current key while retaining predecessors, deploy, wait through the retention window, then remove the old key.

The gateway stores APNs tokens encrypted in PostgreSQL. Database backups therefore contain ciphertext plus authority metadata and must receive the same access controls and retention treatment as the service secrets.

## PostgreSQL and replicas

All replicas must share one PostgreSQL database. Delivery authority, replay admission, and endpoint quota reservation are transactional there, so replica count does not multiply the abuse ceiling. Run only one migration-capable release at a time during schema rollout. Readiness must be removed from load-balancer service endpoints before terminating a pod.

The service reaps expired challenges and replay rows, idle quota rows, expired/revoked delegations, and retention-eligible installations (including their encrypted token ciphertext) at startup and every five minutes. Monitor reaper failures and table growth; retention does not depend on process restarts.

## Metrics and alerting

The gateway serves Prometheus metrics at `GET /metrics` on the **private health listener** (`BUZZ_PUSH_HEALTH_ADDR`, default `0.0.0.0:8081`) — the same port as the probes, never on the public `8080`. All series are sanitized and bounded-cardinality: label values are drawn only from closed sets (the six APNs outcome classes, the fixed admission results, the static error codes already returned to callers, and the readiness causes). No endpoint, device token, relay pubkey, request id, or any request-scoped identifier is ever used as a label.

| Metric | Type | Labels | Meaning |
|---|---|---|---|
| `push_gateway_apns_deliveries_total` | counter | `outcome` = `accepted` \| `invalid_endpoint` \| `retry` \| `refresh_credential` \| `configuration_fault` \| `permanent_request_fault` | Terminal APNs send outcomes. |
| `push_gateway_apns_delivery_seconds` | histogram | — | APNs send round-trip latency (seconds). |
| `push_gateway_apns_credential_refreshes_total` | counter | — | Provider JWT refreshed after APNs reported expiry. |
| `push_gateway_admissions_total` | counter | `result` = `admitted` \| `rejected` \| `unavailable` | Outcome at the `authorize_delivery` replay/quota fence. |
| `push_gateway_delivery_errors_total` | counter | `class` (static) | Selected delivery-handler exit classes only (see note). |
| `push_gateway_reaper_failures_total` | counter | — | Retention reaper sweep failures. |
| `push_gateway_readiness_failures_total` | counter | `cause` = `not_accepting` \| `authority` | Readiness probe failures by cause. |

`push_gateway_delivery_errors_total` is intentionally **narrow**: it counts only selected exit classes of the `/v1/deliveries/apns` handler — `class` ∈ `invalid_grant` (grant rejected at the admission seam, before a permit is issued), `temporarily_unavailable` (authority unavailable at the admission seam), `profile_mismatch`, `token_custody` (endpoint-token open failure), `finish_failed` (detached disposition/join failure returned as 503). Request/auth/attestation/grant validation on the enrollment, delegation, rotation, and revocation handlers is **not** counted by this metric; it is a delivery-hot-path signal, not a total error rate across the API.

Scraping is **opt-in** and off by default, so the default chart render is unchanged and `8081` keeps no pod ingress. To enable it, set `podMonitor.enabled=true` (renders a prometheus-operator `PodMonitor` scraping the `health` port `/metrics`) and `networkPolicy.monitoring.enabled=true` with `networkPolicy.monitoring.namespaceSelector` / `podSelector` naming your scraper — this adds a single `8081` ingress rule scoped to that source, never a blanket allowance. Node/kubelet-origin probe traffic remains exempt from NetworkPolicy regardless.

Alerting rules ship as an opt-in prometheus-operator `PrometheusRule` (`prometheusRule.enabled=true`). Thresholds and operator actions:

| Alert | Fires when | Severity | Action |
|---|---|---|---|
| `PushGatewayConfigurationFault` | any `configuration_fault` outcomes for 10m | critical | APNs provider token/topic is unhealthy. Check the `.p8` key, `BUZZ_PUSH_APNS_KEY_ID`, `..._TEAM_ID`, and `..._TOPIC`. No endpoints are being invalidated, but nothing is delivering. |
| `PushGatewayAdmissionUnavailable` | any admission `unavailable` for 5m | critical | PostgreSQL authority store is unreachable. Check DB connectivity and the pod's `postgresEgressCidrs` NetworkPolicy. |
| `PushGatewayReadinessAuthorityFailing` | readiness `authority` failures for 5m | warning | Replicas are being pulled from the Service on DB check failure. Fix DB health before capacity drops below the PodDisruptionBudget. |
| `PushGatewayReaperFailing` | reaper failed ≥2 times within 30m (runs every 5m) | warning | Expired reservations aren't being swept, growing the bounded-until-expiry window. Check DB write availability. |
| `PushGatewayHighApnsRetryRate` | retryable fraction > `prometheusRule.apnsRetryRatioThreshold` (default `0.25`) over a 10m window, above `apnsRetryMinSamples` (default `20`) attempts, held true for 15m | warning | APNs is throttling or degraded (429/500/503). Deliveries are delayed, not lost. |

## Relay configuration

When the follow-up integration lands, each relay will point `BUZZ_PUSH_GATEWAY_DELIVERY_URL` at the same exact public delivery URL. Relays retain lease matching, authorization, coalescing, durable jobs/retries, and generation checks. They receive only opaque capabilities and never APNs tokens or provider credentials.

## Relay integration status

This PR does **not** enable end-to-end push delivery from a relay. It lands the NIP-PL acceptance and durable lease/outbox primitives plus the independently deployable gateway, but intentionally does not start a relay matcher/worker. The missing client App Attest enrollment/delegation flow must first place a gateway-issued opaque capability—not a raw APNs token—into the encrypted lease. A follow-up must then add per-origin event matching with read-authorization rechecks, durable enqueue, send-time revalidation, and the NIP-98 delivery worker. Operators must leave `BUZZ_PUSH_GATEWAY_DELIVERY_URL` unset until that complete path lands; setting it currently gates lease acceptance only and does not start delivery.

## Helm production inputs

The chart defaults to the `main` image tag because `.github/workflows/docker.yml` publishes it from the push-gateway lane. For a production rollout, open that workflow run's **Build public push gateway image** job summary and copy its `sha256:...` digest. Verify the published subject and provenance before injecting it:

```bash
gh attestation verify \
  oci://ghcr.io/block/buzz-push-gateway@sha256:<64-lowercase-hex> \
  --owner block
```

Only after that command succeeds, set the exact digest as `image.digest`; the chart then renders `ghcr.io/block/buzz-push-gateway@sha256:...` and ignores the mutable tag. `values-production.yaml` is an intentionally invalid production-input contract: deployment CI must inject this verified `image.digest`, the provisioned Apple application identifier, an environment-owned Gateway parent reference, and the actual PostgreSQL network. Schema validation rejects the artifact when any remains empty; the render guard proves both rejection and a fully injected render.

Network policy keeps APNs HTTPS and PostgreSQL egress in separate CIDR lists. APNs currently requires broad TCP/443 reachability; `networkPolicy.postgresEgressCidrs` must be narrowed to the production database network, and the DNS namespace/pod selectors must match the cluster DNS deployment. The sample private CIDR is not a claim about the production topology.

Kubernetes does not restart pods when referenced Secret bytes change. AEAD or APNs credential rotation therefore requires an explicit rolling restart after the secret manager update (for example, `kubectl rollout restart deployment/<release>-buzz-push-gateway`) and readiness verification before removing predecessor keys. Service-account token automount is disabled.
