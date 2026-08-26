# S4 scope — CML workstream view projection

Base: `upstream/main` @ `e8172b5baccd0f4a711f5e19a3bc4313708f1c78`
Branch: `integration/buzz-system-s4` (CML stack replayed clean onto that base)
Status of replay: 3 commits, zero conflicts, 16/16 CML contract tests green.

## Problem this slice solves

`CmlTask.runtime.presence` is validated as a *derived* field, but it is derived
**at `updated_at`** (`cml.rs::validate_presence`). It is a frozen historical
projection, correct only at the instant the transition was signed.

A workstream board renders at *view time*. A task whose last signed transition
carried `presence: Online` still reports `Online` forever, no matter how long
the agent has been dead. Rendering the stored field directly would display a
crashed worker as live. That is the failure mode this slice closes.

## In scope

A pure projection in `buzz-core` turning reduced CML state plus an observation
timestamp into a UI-ready card:

- **Liveness recomputed at observation time** from `last_heartbeat_at` and
  `ttl_seconds`, using the same Online/Stale/Offline thresholds as
  `validate_presence` (`age <= ttl` / `age <= 2*ttl` / else). No heartbeat =>
  Offline.
- **Lease expiry is independent of heartbeat.** An unexpired heartbeat with an
  expired lease is not a live claim.
- **Git metadata** surfaced as `repo`, `branch`, `base_sha`, and short head.
  Absent `head_sha` renders a defined unavailable state, never a fabricated SHA.
- **Privacy:** `worktree_alias` and pseudonymous `host_id` only. No absolute
  paths, no raw IPs, no full pubkeys beyond what CML already carries.

## Explicitly NOT in scope

- Desktop/React rendering, routes, or components (next slice).
- Canvas version history (PR #6780) and the workstream-board stack
  (PRs #6184..#6373). Discovery dispositions recorded in the S4 discovery
  report; no third-party code ported in this commit.
- Any relay, DB, or migration change.
- Mutating stored `presence`; the snapshot field stays as-signed for audit.

## Authoritative vs derived

| Field | Source | Authoritative? |
|---|---|---|
| `title`, `status`, `priority`, `objective` | reduced snapshot | yes |
| `git.*` | reduced snapshot | yes |
| `lease.expires_at` | reduced snapshot | yes |
| `runtime.presence` | signed at `updated_at` | historical only |
| card liveness | recomputed at observation time | derived |

## Acceptance (externally observable values)

1. Heartbeat 60s old, ttl 180, lease valid => liveness `online`, `live_claim` true.
2. Heartbeat 240s old (>ttl, <=2*ttl) => `stale`, `live_claim` false.
3. Heartbeat 400s old (>2*ttl) => `offline`.
4. No heartbeat => `offline`.
5. Heartbeat fresh but lease expired => `live_claim` false.
6. Stored `presence: Online` + stale-by-clock heartbeat => card reports
   `stale`, proving the card does not echo the stored field.
7. Missing `head_sha` => `head_short` is None; no invented value.
