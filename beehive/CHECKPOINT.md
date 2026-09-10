# Beehive implementation checkpoint

Scope: standalone TypeScript experimental package, no existing client changes.
Base origin/main 051c3a270be9c73da9ab06700bcab7d5552fceaa; branch beehive/cbfd9440.

First slice uses a dedicated, isolated loopback TypeScript WebSocket relay. It is
NOT a Buzz/Nostr relay extension: no kind allocations, production policy changes,
or promise of current Buzz relay admission. Ciphertext and signed envelopes are
stored/forwarded by relay, never direct-host HTTP. One locally provisioned owner
Nostr identity is shared by trusted, non-cloned controller/host installations.
Signatures cannot identify individual hosts sharing that key. Local admission
journal is safety state (necessary exception to only runtime/keys local); remote
configuration and receipts are also on relay. No controller service.

Implementation priorities: real network → validated contracts → durable host
admission/outbox → fixture runner lifecycle → line-oriented terminal UI.
Real Buzz Agent / Databricks v2 exact-model evidence is a separate gate; do not
claim fixture success proves authentication, model application or crash safety.
No owner credential stores may be accessed to remove that gate.

Pending parity: guided vendor setup, real ACP catalog/exact-model bridge,
Goose/Claude Code/Codex, ten presets/custom ACP, other Buzz Agent providers,
mesh transport, compute deployment, profiles/metadata, Restart/Move, key import
with saved-owner verification/revocation, service installation, reconnect UX.
No exclusion from requested parity is treated as owner-approved.

## Implemented milestone (2026-09-10)

Executable code: `src/protocol.ts` (runtime codecs, owner Schnorr signatures,
private AES-GCM envelopes), `storage.ts` (0600 atomic/fsynced writes), `relay.ts`
(real loopback WebSocket ciphertext persistence/fanout), `client.ts`, `host.ts`
(fixed local authority binding, exclusive lock, revisioned admission/fingerprints,
outbox, immutable fixture launch, owned process-group Stop), `cli.ts` (local
identity/guided setup, relay/host entry points, line-oriented TUI).

Validation: `npm test` passes 2 tests including actual external runner and
in-group descendant, actual TUI process close/reopen twice, relay client replay,
wrong-agent/revision/fingerprint rejection, exclusive host lock and changed
saved-agent binding rejection, clean host restart retaining stopped assignment.
Node 26.8.1. Fixture success is NOT real harness/model/OAuth success.

`node node_modules/typescript/bin/tsc --noEmit` was attempted and FAILS because
@types/node and @types/ws are unavailable. npm install from registry.npmjs.org
returned Block Cloudflare Dependency Confusion HTML; npm --offline ENOTCACHED.
Runtime tests use ignored symlinks to preinstalled public JS package dependencies
(documented in TESTING.md). No lockfile or clean-install claim. No policy bypass.
Repo-wide `just ci` was not run; no Rust/native/client files changed.

The existing relay explicitly rejects unknown kinds (`handlers/ingest.rs:436`)
and p-gated REQ authorization is kind-specific (`handlers/req.rs:1319`). This
prototype does not presume arbitrary private-kind admission or prove that no
future standard envelope can work; hosted/current-Buzz relay integration remains
an explicit unsolved boundary. Dedicated dev relay is intentionally distinct.

Real target: installed buzz-agent and buzz-acp found under /Applications/Buzz.app;
only an isolated-env auth-help attempt was made, returning DATABRICKS_HOST
required. No OAuth cache, owner key/profile or historical private store read.
Local option 2 records Databricks v2 target and displays blocked readiness;
Start fails closed. README gives the actual local OAuth command and gate.

Fresh-run next step: from `beehive/`, run `npm install && npm run check && npm test`
once approved npm access/types are available; fix all strict diagnostics and
create a lockfile before expanding the ACP bridge. If access remains blocked,
continue with cached-runtime tests and a targeted host-side catalog/exact-model
adapter, never weaken typing or claim model-ready based on spawn.

Known unfinished safety: no crash recovery beyond quarantine/exclusive lock,
no host reconnect, no relay ACK/TUI durable pending-intent store, no outbox
pruning, no OS containment against escaping descendants, no immutable executable
handle/script pinning, no hostile-local-journal validation/migration. No Move,
Restart, key import/revocation, real ACP bridge, multi-agent host, full profile
workflow or vendor parity. README has the concrete parity matrix. This is a
reviewable development checkpoint, not system completion or merge/release-ready.
