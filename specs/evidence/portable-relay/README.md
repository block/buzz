# Portable relay conformance evidence

Black-box Tier-3 runs of the shared conformance vectors produced by
[`cloudflare/portable-relay/scripts/tier3-conformance.mjs`](../../../cloudflare/portable-relay/scripts/tier3-conformance.mjs).
Each JSON file records the capability version, profile, adapter, environment,
fixtures, and per-check outcomes. Evidence contains no private keys, bearer
material, or reusable proofs.

## 2026-07-26 run

- Adapter revision: `76819c48464f3470679795e13e40ee95f937f2e7`
- Cloudflare compatibility date: `2026-07-25`
- Laptop adapter: `buzz-local-relay` release build, fresh ephemeral instances
  (open for core; `--require-auth` for identity)
- Cloudflare adapter: deployed non-production preview on `workers.dev`
  (Worker versions `06be1f2e` open/core, `bc86d919` secured/identity via
  `BUZZ_REQUIRE_AUTH`)

| Suite | Laptop | Cloudflare preview | Outcome parity |
| --- | --- | --- | --- |
| `portable-relay-core-v0.1` (14 checks) | pass | pass | exact |
| `portable-relay-identity-v0.1` (13 checks) | pass | pass | exact |

Local Workers-runtime (Tier 2) coverage additionally proves, with real fault
injection, what a deployed run cannot force: replayed proofs fail closed
across Durable Object eviction, and authenticated WebSocket principals and
subscriptions survive hibernation
(`cloudflare/portable-relay/test/identity/identity.test.ts`).

Known informative-level differences (permitted by the identity profile's
"denial codes normative, messages informative" rule): denial and rejection
message text differs between adapters; JSON object key order differs between
serializers. The runner compares envelopes canonically and outcomes by stable
code.

Deployment-recovery evidence from the 2026-07-25 core run (redeploy of a
compatible revision preserving durable history) was reproduced in this run
implicitly: the secured identity deployment reused the journal written by the
open core deployment.
