# Real host transport — partial tracer checkpoint (NOT deployable)

Base: `419370d38c34f7866e1cbf59b88d9db2bc3756bc`. Isolated branch
`beehive/demo-transport-473233b8`; no moving-branch mesh code included.

## Implemented

- `src/nostr-codec.ts`: pinned `nostr-tools@2.23.1` NIP-44 v2 pairwise encryption,
  signed kind13 seal and fresh ephemeral kind1059 gift wrap. Inner kind14 rumor
  carries JSON Message and a private management subject discriminator; no new
  relay kind allocation. Verifies outer and seal signatures, rumor hash, inner
  sender binding, recipient, domain and existing Message parser. Retains stable
  Message operation ID across fresh wraps. NIP59 library interoperability tested.
- `src/nostr-client.ts`: stable signer NIP42 AUTH, optional OA tag, auth OK before
  REQ, full-history/live subscription before ready (EOSE), EVENT/OK publication,
  64 pending ACK cap, payload/backpressure/startup/ACK bounds, challenge cap,
  subscription CLOSED/socket errors reject pending unknown results. Single
  generation; callers reopen explicitly. No automatic effect retries.
- `src/host-attestation.ts`: public verification and explicit owner-side signing
  primitive matching the upstream OA preimage and canonical conditions. Includes
  published upstream SDK signature vector. No credential harvesting or key daemon.
- `src/host-identity.ts`: owner-public-only durable independent host key, pending
  by default, genuine OA import; private atomic writes and the existing host.lock
  discipline. Refuses legacy setup.json and never migrates shared owner secrets.
- `authorizeManagement`: narrow tracer authority policy; owner commands must prove
  owner and target label; inventory/receipts must prove exact supplied host key.
  ALL host-to-host Move messages are denied pending real catalog/assignment audit.
  The supplied catalog is a caller trust input, NOT yet an owner-attested record.

No existing lifecycle/conversation code or normal CLI behavior was changed. These
are implemented seams awaiting integration, NOT a repaired user-facing demo.

## Source proof (not deployed policy)

Pinned upstream: `block/buzz@051c3a270be9c73da9ab06700bcab7d5552fceaa`.

- `crates/buzz-relay/src/handlers/ingest.rs:2203–2280`: WebSocket-only gift wraps;
  event signature validation; +/-900s timestamp; 256KB content; **explicit**
  gift-wrap exception to author/authenticated-connection identity equality;
  required scope comes from authenticated context (`MessagesWrite`, lines 471–472).
- `handlers/auth.rs:218–267`, `api/mod.rs:191–210`: NIP-OA checked on stable
  identity's signed AUTH for ViaOwner membership. The ephemeral wrapper does not
  need its own OA; its signature proves only ciphertext integrity, NOT authority.
- `buzz-sdk/src/nip_oa.rs:289–320`: AUTH verifies signature and temporal clauses,
  deliberately ignoring `kind=`. Therefore `kind=1059` OA must NOT be described
  as a management-command or ephemeral-wrapper authorization restriction.
- `buzz-core/src/kind.rs` P_GATED_KINDS and `handlers/req.rs` p-gated filter policy:
  recipient-scoped giftwrap reads. Fixture enforces exact self #p filter.
- `desktop/src-tauri/src/commands/agents.rs:433–450`: owner signing is coupled to
  managed-agent creation; no verified arbitrary-host-public-key minting UI/CLI
  entrypoint was found. Do not create an agent draft to masquerade as a host.

Crypto correction to the gap memo: NIP44 v2 uses ChaCha20 + HMAC-SHA256 with HKDF,
not ChaCha20-Poly1305. Production code imports the tested library, not handrolled
primitives. NIP59 convenience unwrap is insufficient as an authentication API
(it decrypts without the explicit seal/rumor verification implemented here).

Timestamp decision: current timestamps instead of NIP59 recommended random
backdating meet the pinned relay admission window. This is a documented privacy
tradeoff, not transparent conformance to its backdating recommendation. Operation
identity is Message.id, not any outer/seal/rumor event ID.

## Actual validation

Node24.15.0: `/Users/loganj/Library/Caches/hermit/pkg/node-24.15.0/bin/node`.
Standalone pnpm11.4.0 `dist/pnpm.mjs`, approved mirror, frozen lock install,
no workspace bootstrap or borrowed dependencies. Direct package-local tsc.

- Strict TypeScript: PASS.
- `node --test test/nostr-transport.test.ts`: **6/6 PASS**.
- Tests exercise real WS frames against a narrow executable admission model in
  `test/nostr-fixture.ts`: owner + two independent host identities, genuine
  fixture OA, AUTH, p-gating, inventory, valid owner Start message, host receipt,
  connection reopen/history, fresh wrap same inner operation ID. Negative tests:
  unsigned AUTH, no enrollment, timestamp outside admission, unsupported 30179,
  altered signature/ciphertext, wrong recipient/rumor author, cross-host/owner
  spoofing, denied Move grants, durable pending/import/lock behavior.
- Published NIP-OA SDK vector passes. NIP44/NIP59 interoperability with library
  tested; official NIP44 vector corpus is NOT independently run in this checkpoint.
- This fixture models the cited policy subset, not the actual Rust relay/server
  stack, token scopes, DB materialization, query pagination or deployed settings.
- Rewrap dedup test establishes equal inner IDs only; it deliberately does NOT
  claim durable lifecycle effect dedup. No Start/Move execution over new transport.
- ONE full default-concurrent installed-enabled suite attempted with inherited
  BUZZ_PRIVATE_KEY/BUZZ_AUTH_TAG/BUZZ_RELAY_URL removed and pinned Node PATH.
  **NOT PASS**: log has 79 passing test records, one failure named
  `same-batch Stop cancellation keeps authority, ordering, duplicate and revision fences`,
  and no final summary before the tool's 120s bound. No rerun, serial mask,
  timeout inflation, force-exit or latency classification. Failure details were
  not printed before cutoff. Preserve this unresolved result.

Installed binary SHA256 (separate from candidate TS source):

- buzz-acp: `10612d0025d1420bdd9e9afcc2e7129377da2414e75049a8b640e81d857441ea`
- buzz: `147cc2ccf276ddedc5b84d13f5399a95282066303c9dca566bb2af5a37b856c5`

Both from `/Applications/Buzz.app/Contents/MacOS/`; invoked only by fresh existing
fixture journeys. No owner profiles, live login/refresh/inference or live probes.

## Self-review and required continuation

**Security review gate remains CLOSED. Not a release candidate.**

1. Integrate at connect/intents/host boundaries, not a parallel agent runtime.
   Existing journals currently store legacy encrypted envelopes; replace durable
   transport persistence with stable signed message identity plus fresh wrapping
   while preserving exact fingerprint/CAS/cancellation/replay receipt semantics.
2. Owner-attested public host catalog and cryptographic host binding must replace
   caller-supplied label mapping before trust. Move source/target authority needs
   exact target/agent/fingerprint/retained-assignment verification; source-consumed
   authority/outbox survives restart and target assigns before launch validation.
   Do not relax current grant denial until this is implemented and tested.
3. Integrate bounded reconnect and policy-close recovery with existing durable
   unknown-vs-accepted UI; current single-generation seam only exposes explicit
   reopen. Full history queries may require production pagination reconciliation.
4. Normal CLI/local wizard must call owner-public-only bootstrap and genuine
   attestation import. Current normal setup still uses legacy development transport;
   do not deploy this branch or run normal setup for the requested real-host demo.
   No desktop/laptop product setup commands are supported yet.
5. Provide explicit owner-side interactive signer entrypoint or use a verified
   existing owner signer API. Current attestHost is only a library primitive.
   Do not distribute/store owner secrets on hosts, copy Larry's auth tag, create
   controller authority, or use fabricated live attestations.
6. Extend production-shaped tests through actual Start/reply/Move/same-agent reply
   with installed Buzz + TS provider fixture. Exercise persistent lifecycle dedup,
   cancellation, backpressure/closed/rechallenge/reconnect and catalog spoofing.
   Fix/classify full-suite result without rerun-to-green; no readiness claim.
7. Independent security review at immutable pin BEFORE any install. Only after
   review and legitimate owner authorization: enroll actual host keys and verify
   deployed ViaOwner/MessagesWrite policy. Source proof is not live policy proof.

Next executable developer action (not operator deployment):

```
cd /Users/loganj/.buzz/REPOS/beehive-demo-transport-473233b8/beehive
/Users/loganj/Library/Caches/hermit/pkg/node-24.15.0/bin/node --test test/nostr-transport.test.ts
```

Then implement the retained-message/host-key catalog integration above on this
single WIP branch. No mesh, existing native client edits, release migration,
repository migration, PR or deployment is authorized by this checkpoint.
