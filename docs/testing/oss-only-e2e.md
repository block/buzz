# OSS-only authorization and lifecycle playground

This playground exercises the public provider/evidence seam and the disabled
operator composition root with synthetic data only. It starts isolated
PostgreSQL, Redis, and MinIO services under the `buzz-oss-e2e` Compose project,
then builds and starts two real stock relay processes on loopback ports. It does
not register operator routes in either relay, grant operator authority, connect
to a private identity system, or use deployment data.

## Quick start

```sh
just oss-e2e-setup
scripts/oss-e2e.sh scenario TOPOLOGY
just oss-e2e-scenario O501
just oss-e2e-scenario P01
just oss-e2e-stop
```

`just oss-e2e` runs the live topology and the complete focused scenario table,
writes a bounded JSON scenario summary under the process-specific temporary
state directory, and stops every process it started. `just oss-e2e-reset`
deletes only the local `buzz-oss-e2e` Compose project's synthetic volumes and
starts a fresh stack. Formal schema setup always uses the embedded SQLx
migration chain; the playground never imports a handwritten test schema.

Database-backed scenarios fail when PostgreSQL is unavailable. The O5 tests
also assert that exactly 50 gap-free migrations ran and that their scenario
counters are nonzero, so unavailable infrastructure cannot produce a vacuous
green result.

The `TOPOLOGY` scenario is a real process/network gate, not a list of unit-test
aliases. It verifies all 50 applied SQLx checksums against the embedded
migrator, publishes over relay A while a WebSocket client on relay B observes
Redis fan-out, uploads media through A and downloads it through B, performs a
real Git clone/push/clone round trip, exchanges a v2 audio frame between two
clients, restarts relay B, and queries the persisted event from the restarted
process. It also plants synthetic secret canaries and scans public errors,
metrics, and both relay logs for disclosure.

## Scenarios

| ID | Expected outcome |
| --- | --- |
| A01 | A current, domain-bound provider decision produces the requested scoped allow snapshot. |
| D01 | Provider unavailability never falls back to allow. |
| D02 | Duplicate or unknown domain configuration is rejected as ambiguous. |
| D03 | Stale and future provider decisions deny. |
| D04 | A proof bound to the wrong authorization domain is rejected before authority I/O or mutation. |
| L01 | A durable principal tombstone blocks first enrollment and re-enrollment. |
| L02 | A protected authorization lease ends at the earliest binding/application expiry. |
| L03 | Rotation/revocation projection retries after restart and publishes one canonical withdrawal. |
| R01 | Restart bootstraps the complete durable invalidation state before readiness. |
| O501 | Explicit authenticated composition reaches list, preview, revoke, and rotate; outbox rollback, ordered retry, quarantine, restoration, and capacity cases execute against PostgreSQL. |
| P01 | Planted token, JWT, issuer, JWKS-body, display-claim, and private-identifier canaries are absent from client errors, tracing, metrics, immutable audit, export bytes, and dead-letter evidence. |

The machine-readable topology summary also names `M01` (migration checksums),
`H01` (HTTP/media), `G01` (Git), and `AU01` (audio) so a consumer can distinguish
the live client surfaces from the focused contract cards above.

Run one scenario directly with:

```sh
scripts/oss-e2e.sh scenario D03
```

## Operator boundary

The stock binary installs no lifecycle routes. The test-only composition root
requires an explicitly supplied authenticator, capability grant, executor,
clock, pseudonymization key, and operator-reference key. The PostgreSQL route
test sends authenticated synthetic requests through that real composition root
and verifies durable operation receipts and effects. Direct storage calls alone
do not count as route evidence.

List and preview return redacted opaque references. Revoke and rotate require a
reason, operation and correlation identities, matching intent, a fresh
independent approval, and the expected lifecycle revision. An unavailable
audit store blocks a new allow or operator mutation. A denial remains a denial
and emits a separate bounded control signal.

## Data and cleanup

The playground binds only to local high ports: relay A `3301`, relay B `3302`,
health `8301`/`8302`, metrics `9301`/`9302`, PostgreSQL `5546`, Redis `6546`, and
MinIO `9546`/`9547`. Its credentials are fixed synthetic test strings. Each O5
PostgreSQL test creates a uniquely named disposable database, applies migrations
`0001` through `0050`, and drops that database after success. The live topology
uses the Compose project's synthetic `buzz` database and verifies every applied
checksum before driving clients.

Use `just oss-e2e-stop` to stop services while retaining synthetic volumes, or
`just oss-e2e-reset` for a destructive reset limited to this Compose project.
