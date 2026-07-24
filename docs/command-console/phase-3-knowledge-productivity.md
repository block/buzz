# Command Console Phase 3 knowledge and productivity

Phase 3 gives the macOS Command Console three local, read-only evidence
sources: a signed RAG mirror, a writable local Memory MCP node with
conflict-safe home replication, and allowlisted Apple productivity inputs.
The Tauri application admits those sources only after cryptographic and
structural validation and then projects an exact loopback-only MCP catalogue
to the Phase 2 LM Studio runtime.

This phase does not generate the Daily Command Brief, schedule adviser runs,
route `PUBLIC` work to cloud models, or mutate the Buzz workspace. Those remain
later phases. The application is advisory and is not an accredited operational
or navigation system.

## macOS application boundary

The product remains a Tauri/React macOS application. A native Swift helper is
built by the checked-in Xcode project at
`desktop/apple-inputs/BuzzAppleInputs.xcodeproj`. It owns EventKit access for
Calendar and Reminders, the constrained Notes bridge, and bounded file reads.
The Tauri process owns configuration, Keychain access, SSH pinning, Memory
replication, RAG admission, source-status persistence, and the policy supplied
to the LM Studio ACP runtime.

The helper accepts only its closed newline-delimited JSON protocol. It does not
accept arbitrary AppleScript, shell commands, directories, or filesystem
roots. Calendar IDs, reminder-list IDs, Notes folders, and individual file
paths are caller allowlists. Files are opened beneath the compiled allowed
roots without following symlinks and must be bounded regular UTF-8 files.

Permission denial, stale data, a deleted item, or one unavailable source is a
degraded section rather than permission to synthesize missing evidence. The
Command Console shows the observed permission, record count, truncation state,
and fixed error code for each source.

Run the helper contract independently with full Xcode:

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  xcodebuild test \
    -project desktop/apple-inputs/BuzzAppleInputs.xcodeproj \
    -scheme BuzzAppleInputs \
    -destination 'platform=macOS' \
    CODE_SIGNING_ALLOWED=NO
```

Release distribution still requires the normal signed and notarised Buzz
bundle. `CODE_SIGNING_ALLOWED=NO` is a test setting, not a deployment setting.

## Memory MCP local authority

The MacBook-local Memory MCP node is the command application's writable
authority. Its Markdown vault and immutable replication objects are durable;
the SQLite search index is rebuildable. Buzz writes locally and synchronises
with the home node only through a pinned SSH host identity and a reserved
literal-loopback tunnel.

The local Compose profile is opt-in. It publishes Memory only on
`127.0.0.1`, mounts separate vault and index volumes, requires MCP
authentication, and reads two protected files:

- a JSON map from random bearer values to capability arrays, such as
  `{"<read>":["read"],"<sync>":["replicate"]}`;
- a separate HMAC attestation secret.

Configure their paths outside source control:

```dotenv
BUZZ_MEMORY_MCP_IMAGE=memory-mcp:reviewed
BUZZ_MEMORY_PORT=18006
BUZZ_MEMORY_NODE_ID=node:macbook-command
BUZZ_MEMORY_REPLICATION_TOKENS_FILE=.local/memory-replication-tokens.json
BUZZ_MEMORY_ATTESTATION_SECRET_FILE=.local/memory-attestation-secret
```

Both files must be mode `0600`. The attestation secret must be independent of
every bearer value. Memory MCP and Buzz both reject malformed credentials and
equal bearer/attestation values.

Place `command-memory.json` in Buzz's application configuration directory. All
paths are absolute and protected, node IDs are stable and distinct, and
credential values are referenced by Keychain key rather than stored here:

```json
{
  "schema_version": 1,
  "local_port": 18006,
  "home_host_alias": "memory-home",
  "home_user": "memory-sync",
  "pinned_host_fingerprint": "SHA256:reviewed-host-key-fingerprint",
  "known_hosts_path": "/protected/buzz/memory_known_hosts",
  "identity_file": "/protected/buzz/memory_sync_ed25519",
  "remote_loopback_port": 8006,
  "local_node_id": "node:macbook-command",
  "home_node_id": "node:home-command",
  "sync_interval_minutes": 30,
  "tool_allowlist": [
    "command_memory_context",
    "recall_for_entity",
    "search_events",
    "record_event"
  ],
  "credential_keys": {
    "local_read": "memory.local.read",
    "local_attestation": "memory.local.attestation",
    "local_replicate": "memory.local.replicate",
    "remote_read": "memory.remote.read",
    "remote_replicate": "memory.remote.replicate"
  }
}
```

Buzz's shared `SecretStore` JSON blob uses Keychain service `buzz-desktop` in
production and `buzz-desktop-dev.<instance>` in development, account
`secrets`. Provision keys through a trusted SecretStore path that preserves
the other entries. Do not replace the shared blob with
`security add-generic-password`.

Replication exchanges bounded immutable events and parent-addressed entity
revisions with globally unique IDs and resumable cursors. Append-only events
deduplicate. Divergent stable-entity heads create a visible conflict; neither
side wins by timestamp. Conflicted fields, tombstones, empty revisions, and
oversized values are excluded from unattended adviser evidence.

`command_memory_context` is the sole Memory tool admitted to an adviser. It
returns bounded current-head `memory-evidence-v1` records with the exact
revision, replication envelope, quoted content, origin node/time, serving
node/time, revision hash, and journal cursor. Retrieved text is explicitly
untrusted evidence with no instruction effect. Other read tools remain
available to the Command Memory workflow; write tools require explicit
`read` plus `admin` capability and are not in the adviser catalogue.

## Signed local RAG mirror

The home RAG corpus remains authoritative. Its export is a signed,
content-addressed bundle containing Qdrant snapshots, exact collection
schema/counts, document catalogue, RAG service commit, dense `bge-m3`, sparse,
and reranker identities, golden queries, timestamps, and checksums. Copying
Qdrant data alone is not a valid mirror.

Import is staging-only until all of these succeed:

1. Ed25519 signer, manifest canonicalisation, and declared object graph;
2. streamed object checksums and archive safety bounds;
3. disk reservation for candidate, restore, rollback, and 20% remaining free
   space;
4. disposable Qdrant restore with exact schema and point counts;
5. exact dense, sparse, reranker, and service revisions; and
6. approved golden retrieval queries through the candidate pipeline.

Activation writes an immutable activation record and atomically switches a
relative `active` pointer. A prepared but incomplete activation rolls back on
recovery. The prior verified activation is retained for explicit rollback.
Every search holds one activation from collection resolution through evidence
formatting, so a result cannot mix snapshots.

The macOS RAG Compose topology publishes only
`http://127.0.0.1:8005/mcp/`. Qdrant, dense, sparse, and reranker services are
internal. Images must be reviewed immutable digest references, model files are
pre-seeded, telemetry/offline flags are enabled where supported, and the
OFFICIAL retrieval service exposes no legacy raw search, chat, upload, delete,
or activation endpoint.

Place `command-rag.json` in Buzz's application configuration directory:

```json
{
  "schema_version": 1,
  "endpoint": "http://127.0.0.1:8005/mcp/",
  "state_root": "/protected/command-rag",
  "expected_server_identity": "rag",
  "expected_active_snapshot_id": "<64-lowercase-hex>",
  "trusted_signer_fingerprint": "<64-lowercase-hex>",
  "credential_key": "rag.local.read",
  "attestation_credential_key": "rag.local.attestation",
  "tool_allowlist": [
    "search_knowledge_base",
    "list_collections",
    "get_document",
    "get_snapshot_status"
  ],
  "maximum_snapshot_age_hours": 48
}
```

The two referenced Keychain values must be independent. RAG and Buzz both
reject equal bearer/attestation values. Buzz re-hashes the local manifest and
activation, verifies the signer and active snapshot, probes an HMAC challenge,
and requires the exact tool catalogue before caching admission for 30 seconds.
Changing the active snapshot invalidates the configured identity until the
operator reviews and updates the expected snapshot.

RAG search returns `rag-evidence-v1`. Every quoted result binds source,
collection, document/chunk, active snapshot, retrieval time, quoted location,
and score components. The quoted passage is untrusted evidence and cannot
issue instructions.

## Admission into LM Studio

Readiness is not admission. A service enters
`LM_STUDIO_MCP_INTEGRATIONS` only after all applicable checks pass:

- canonical literal-loopback endpoint and exact path;
- valid bearer loaded from Keychain;
- a separate valid HMAC attestation secret;
- expected service and active node/snapshot identity;
- exact tool catalogue and workflow allowlist;
- local signed state, freshness, and source-schema validation; and
- bounded, authenticated service responses.

The Memory adviser catalogue contains only `command_memory_context`. The RAG
catalogue contains only the four read tools above. A prompt, persona, retrieved
document, free-form environment variable, or LAN endpoint cannot add a tool or
change a URL. A malformed native MCP result rejects the whole model response
before its `response_id` can enter continuation state.

The status card reports Memory local/home IDs, both cursors, conflicts, last
successful sync, and `fresh`, `stale`, `never_synced`, or `corrupt`; RAG
reports active snapshot, signer, activation, validation, and freshness; Apple
inputs report permission and collection state. A failed source is removed from
the model catalogue and shown as degraded.

## Hermetic acceptance

Run the Buzz-side Phase 3 gate:

```bash
source bin/activate-hermit
just check-command-knowledge
```

It covers the Compose security contract, LM Studio fixtures, portable Memory
hashing, evidence rejection, Memory replication and persisted status, Memory
and RAG admission, Command Console presentation, and the Xcode helper. The
orchestration test proves that any failed sub-check suppresses the success
claim:

```bash
./scripts/tests/check-command-knowledge-test.sh
```

The companion repositories retain their own full gates:

```bash
# AgentMemory
cd MemoryMCPServer
.venv/bin/pytest

# RAG-MCP
PYTHONPATH=.:retrieval .venv/bin/pytest -q tests
```

Hermetic acceptance proves code and protocol behavior with bounded fake
services. It does not prove a live deployment.

## Deployment acceptance still required

Before real `OFFICIAL` material is introduced, a controlled operator exercise
must still:

- export a real home bundle while ingestion is paused and retain the prior
  bundle;
- stage and activate it on the MacBook with reviewed images, real keys, exact
  model identities, point counts, and golden queries;
- provision protected Buzz configuration, Keychain values, SSH identity, and
  pinned home host key;
- compare approved navigation golden queries between home and Mac services;
- exercise concurrent changes, duplicate delivery, interruption, conflicts,
  tombstones, backups, restore, and rollback;
- deny internet and home LAN, restart Docker, LM Studio, Buzz relay, Memory,
  RAG, and the macOS app, then prove local Memory read/write, RAG retrieval,
  Apple fail-soft inputs, and LM Studio tool calls;
- restore Buzz, Memory, and RAG under a clean macOS test profile; and
- complete Defence information-handling, host-egress, signing/notarisation,
  and security review.

No live signed home bundle import, air-gapped restart, or clean-profile restore
is claimed by this phase's repository tests.
