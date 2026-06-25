# Buzz Security-Relevant Architecture Report

This report explains Buzz from a security review point of view. It is based on
the current code in this repository, including work in progress areas such as
managed agents, relay membership, workspace switching, Git over the relay, and
multi-tenant direction.

The short version: Buzz is a Nostr-backed collaboration system. Almost every
user action becomes a signed Nostr event. The relay verifies signatures, checks
membership, writes events to Postgres, fans them out over WebSockets, and sends
selected content to search, audit, workflow, media, Git, or audio subsystems.
The main security boundaries are Nostr private keys, relay-level membership,
channel membership, event kind validation, and the process boundary around
agents.

## Code Layout

The security-relevant code is spread across the relay, shared Nostr logic,
clients, agent harness, and supporting services.

Relay and core:

- `crates/buzz-relay/src/main.rs` starts the relay, enforces production key
  requirements, and wires services together.
- `crates/buzz-relay/src/config.rs` reads environment configuration and
  service secrets.
- `crates/buzz-relay/src/state.rs` holds shared runtime state, caches, queues,
  the relay keypair, and service clients.
- `crates/buzz-relay/src/connection.rs` handles WebSocket connection lifecycle,
  frame limits, heartbeats, and handler concurrency.
- `crates/buzz-relay/src/handlers/auth.rs` implements NIP-42 WebSocket auth.
- `crates/buzz-relay/src/handlers/event.rs` handles incoming WebSocket `EVENT`
  messages and live fanout.
- `crates/buzz-relay/src/handlers/req.rs` and
  `crates/buzz-relay/src/handlers/count.rs` implement query and count behavior.
- `crates/buzz-relay/src/handlers/ingest.rs` is the main persistent event ingest
  path used by WebSocket and HTTP.
- `crates/buzz-relay/src/handlers/side_effects.rs` applies channel membership,
  channel metadata, deletion, discovery, and notification side effects.
- `crates/buzz-relay/src/handlers/command_executor.rs` handles command events
  such as workflow creation, workflow approval, and direct message commands.
- `crates/buzz-core/src/kind.rs` defines the event kind registry.
- `crates/buzz-core/src/verification.rs` verifies Nostr event IDs and Schnorr
  signatures.
- `crates/buzz-core/src/filter.rs` implements filter matching and per-event
  reader authorization.
- `crates/buzz-auth/src/nip42.rs` and `crates/buzz-auth/src/nip98.rs` implement
  NIP-42 and NIP-98 authentication.

Data and supporting services:

- `crates/buzz-db/src/*` contains Postgres data access for events, channels,
  members, API tokens, workflows, relay members, and user state.
- `migrations/0001_initial_schema.sql` defines the main storage model.
- `crates/buzz-search/src/lib.rs` configures Typesense search.
- `crates/buzz-audit/src/*` records audit log entries.
- `crates/buzz-media/src/*` validates and stores uploads in S3-compatible
  storage.
- `crates/buzz-relay/src/api/media.rs` exposes Blossom-style media upload and
  download endpoints.
- `crates/buzz-relay/src/api/bridge.rs` exposes HTTP `/events`, `/query`,
  `/count`, and workflow webhook endpoints.

Clients and identity:

- `desktop/src-tauri/src/app_state.rs` creates and stores the desktop user
  Nostr key.
- `desktop/src-tauri/src/commands/identity.rs` exposes signing, import, export,
  and auth event commands to the Tauri frontend.
- `desktop/src-tauri/src/commands/workspace.rs` applies the active relay
  workspace to the backend.
- `desktop/src/features/workspaces/*` stores workspace metadata in frontend
  storage and forces React remounts during workspace switching.
- `mobile/lib/shared/workspace/*` stores mobile workspace configuration,
  including the mobile user private key.
- `mobile/lib/shared/auth/auth_provider.dart` authenticates the mobile client
  against the relay.

Agents and developer tools:

- `desktop/src-tauri/src/managed_agents/*` creates, stores, launches, stops, and
  logs managed agents.
- `crates/buzz-acp/src/*` is the ACP harness that bridges Buzz events to an AI
  agent process.
- `crates/buzz-agent/src/*` is a minimal ACP-compatible agent.
- `crates/buzz-dev-mcp/src/*` is the local developer MCP server. It exposes
  shell and file-editing tools to agents.

Other important feature areas:

- `crates/buzz-workflow/src/*` executes workflow definitions.
- `crates/buzz-relay/src/api/git/*` implements Git smart HTTP, object storage,
  and push policy checks.
- `crates/git-credential-nostr/src/lib.rs` signs Git HTTP credentials with a
  Nostr key.
- `crates/git-sign-nostr/src/lib.rs` signs Git commits and tags with Nostr keys.
- `crates/buzz-core/src/pairing/*`, `crates/buzz-pair-relay/src/lib.rs`, and
  `crates/buzz-pairing-cli/src/main.rs` implement device pairing.
- `crates/buzz-relay/src/audio/*` implements huddle audio rooms.
- `crates/buzz-proxy/src/*` provides Nostr compatibility proxy behavior.

## System Model

Buzz uses Nostr as the event format and identity model. A user or agent has a
Nostr keypair. The public key is the account identifier. The private key signs
events.

The relay is the central service. It accepts WebSocket and HTTP requests,
verifies events, checks authorization, writes accepted events to Postgres, and
fans events out to live subscribers. Redis is used for pub/sub and presence.
Typesense is used for search. S3-compatible storage is used for media and Git
object data.

The relay is not a gossiping Nostr relay. It does not replicate events to other
relays as a normal Nostr network relay would. The security model is closer to a
single workspace service that uses Nostr events as signed records.

The important runtime boundaries are:

- Relay process boundary: all normal clients and agents must go through relay
  validation.
- User key boundary: whoever controls a user's Nostr private key can act as
  that user.
- Agent key boundary: a managed agent has its own Nostr key. The desktop app
  can delegate owner context to it with a NIP-OA auth tag.
- Channel membership boundary: private channel reads and writes depend on
  membership and role checks.
- Relay membership boundary: when enabled, the relay only admits configured
  relay members or delegated agents whose owners are relay members.
- Local machine boundary: managed agents and the dev MCP server can run local
  commands. That is intentionally powerful and should be treated like local code
  execution.

## Nostr Machinery

### Event Identity And Signatures

Nostr events contain a public key, kind, timestamp, tags, content, ID, and
signature. The event ID is derived from the event body. The signature is a
Schnorr signature over that ID.

The relay verifies both the event ID and the signature in
`crates/buzz-core/src/verification.rs`. The persistent ingest path in
`crates/buzz-relay/src/handlers/ingest.rs` rejects events that fail signature
verification. The WebSocket event handler in
`crates/buzz-relay/src/handlers/event.rs` also rejects events whose `pubkey`
does not match the authenticated connection, except for specific proxy and gift
wrap cases.

This is a strong property: accepted application events are attributable to the
Nostr key that signed them. The sharp edge is also simple: key compromise is
account compromise.

### Event Kinds

All known event kinds are registered in `crates/buzz-core/src/kind.rs`. Unknown
kinds are rejected by the relay ingest path. This is good because it keeps the
relay from silently storing unexpected signed data with unclear semantics.

Some important kinds:

- NIP-42 auth: `22242`
- NIP-98 HTTP auth: `27235`
- Blossom upload auth: `24242`
- Gift wrap: `1059`
- Channel messages and NIP-29 group events
- Channel metadata and discovery events: `39000`, `39001`, `39002`
- Membership notifications: `44100`, `44101`
- Presence and typing ephemeral events: `20001`, `20002`
- Agent observer frames and agent-specific events
- Workflow, approval, Git, and relay administration events

The relay has several kind-specific guardrails in
`crates/buzz-relay/src/handlers/ingest.rs`: relay-only events are rejected from
clients, HTTP-only or WebSocket-only restrictions are enforced, delete events
must be structurally valid, encrypted engram/reminder envelopes are checked for
basic shape, and channel-scoped kinds must carry valid channel scope.

### WebSocket Auth: NIP-42

WebSocket connections authenticate with NIP-42. The relay sends a random
challenge. The client signs an auth event with the challenge and relay URL. The
relay verifies the signature, the challenge, the relay tag, and timestamp
freshness in `crates/buzz-auth/src/nip42.rs`.

The connection-level auth handler is in `crates/buzz-relay/src/handlers/auth.rs`.
After NIP-42 succeeds, the relay can also check:

- Pubkey allowlist, if configured.
- Relay membership, if `BUZZ_REQUIRE_RELAY_MEMBERSHIP=true`.
- NIP-OA delegation, if `BUZZ_ALLOW_NIP_OA_AUTH=true`.

When relay membership is enabled, `RELAY_OWNER_PUBKEY` bootstraps the owner and
`BUZZ_RELAY_PRIVATE_KEY` is required at startup. This is enforced in
`crates/buzz-relay/src/main.rs`.

### HTTP Auth: NIP-98

HTTP requests use NIP-98 signed auth events. Verification is in
`crates/buzz-auth/src/nip98.rs`; bridge route handling is in
`crates/buzz-relay/src/api/bridge.rs`.

The auth event binds the signer, URL, method, timestamp, and optionally a
payload hash. Buzz clients generally include a payload hash for normal JSON
requests. The verifier accepts requests without a payload hash if the auth event
does not include one. That is allowed by the NIP, but it is risky for
state-changing non-streaming requests because the body is not cryptographically
bound to the signature.

The HTTP bridge has a replay cache for NIP-98 event IDs. Git HTTP intentionally
uses a different tradeoff, described later.

### Scopes, Tokens, And Real Authorization

Buzz has a `Scope` model in `crates/buzz-auth/src/scope.rs`. API tokens can be
stored with scoped permissions in the `api_tokens` table. Those token records
are used by paths such as production media upload.

For normal Nostr-authenticated users, the relay generally grants the known
application scopes after authentication and then relies on relay membership,
channel membership, ownership checks, and per-kind validation for the real
authorization decision. That is important during review: a scope check may be
necessary, but it is rarely sufficient by itself.

This also means new features should not treat "authenticated Nostr user" as
"authorized user." They need an explicit policy for the event kind, channel,
global record, owner, or resource being accessed.

### NIP-OA Delegation

Buzz uses NIP-OA-style auth tags to connect an agent key to a human owner key.
The helper is in `crates/buzz-sdk/src/nip_oa.rs`. The owner signs an auth tag
that names the owner pubkey, the agent pubkey, conditions, and a signature. The
relay can verify that tag when `BUZZ_ALLOW_NIP_OA_AUTH=true`.

This is how a managed agent can be admitted when relay membership is enabled and
the owner is a relay member. The agent still signs events with its own key. The
auth tag proves that the owner delegated that agent.

The sharp edge is that `BUZZ_AUTH_TAG` is a credential. It is not the owner's
private key, but when paired with the agent private key it can carry owner
context in parts of the system that trust NIP-OA. If conditions gain more
semantics, expiry, or scope rules, those rules need to be enforced everywhere
the tag is accepted.

### Encrypted Nostr Payloads

Some Buzz data is encrypted at the Nostr payload layer. Gift wraps, engrams,
event reminders, observer frames, and pairing messages all use encrypted content
or encrypted envelopes in different ways.

The relay can validate envelope shape, route by tags, and enforce read gates,
but it usually cannot read the encrypted plaintext. The metadata is still
visible: kind, pubkey, created timestamp, tags, event size, and routing
relationships. NIP-44 conversation keys are derived in memory from Nostr keys;
they are not stored as independent long-lived keys in the relay.

The important review rule is that encryption does not remove the need for relay
authorization. Encrypted global events still need p-gates, author gates,
indexing exclusions, and live fanout filtering.

### Channel Scope

Buzz uses the NIP-29 `h` tag for channels. Channel data is not scoped by `e`
tags. `h` tags and `channel_id` are checked throughout the relay.

Important code:

- `crates/buzz-relay/src/handlers/ingest.rs` resolves channel scope and checks
  membership before storing most channel events.
- `crates/buzz-relay/src/handlers/side_effects.rs` applies role checks for
  admin actions.
- `crates/buzz-relay/src/handlers/req.rs` checks channel access before
  registering subscriptions or returning historical results.
- `crates/buzz-core/src/filter.rs` performs final filter matching and
  per-event reader authorization.

Private channel reads and writes require membership. Open channels are
deliberately readable and writable by nonmembers, subject to relay-level auth
and any token restrictions. If relay membership is disabled, an arbitrary
authenticated Nostr identity can interact with open channels.

### Global Events And P-Gated Events

Some events are global instead of channel-scoped. Examples include profiles,
gift wraps, user state, agent engrams, reminders, and some Git or workflow
events.

Global events are a security sharp edge because they are not protected by
channel membership. The relay compensates with kind-specific checks:

- Gift wraps and DM visibility records are p-gated. Queries must include a
  `#p` tag that matches the authenticated user.
- Agent engrams must be requested by the author or by the user named in `#p`.
- Author-only kinds, such as event reminders, can only be queried by the author.
- Search excludes gift wraps, DM visibility records, and author-only records.

The relevant checks live in `crates/buzz-relay/src/handlers/req.rs`,
`crates/buzz-relay/src/handlers/event.rs`, and
`crates/buzz-core/src/filter.rs`.

One sharp edge: global-only kinds are stored with `channel_id = NULL`, but the
raw signed tags are still present. `crates/buzz-core/src/filter.rs` treats
explicit `h` tags as authoritative during filter matching. A malformed or
surprising global event with an `h` tag can therefore match a channel-style
filter even though it is not stored as a channel event. This should not leak
private channel data by itself, because the event is global already, but it can
confuse automation, counts, or UI assumptions.

### Live Fanout And Queries

Subscriptions are registered after auth and access checks in
`crates/buzz-relay/src/handlers/req.rs`. The subscription registry in
`crates/buzz-relay/src/subscription.rs` keeps channel and global subscriptions
separate. Channel subscriptions do not receive global events, and global
subscriptions do not receive channel events.

Live fanout also has a backstop in `crates/buzz-relay/src/handlers/event.rs`.
Private channel fanout is filtered by visibility and membership caches before
events are sent. Sensitive global kinds are filtered by recipient. Search
results are also post-filtered after Typesense returns matches.

This is one of the stronger parts of the system: authorization is checked both
before registering interest and before sending results.

### Replaceable Events, Deletions, And Side Effects

Nostr has replaceable and parameterized replaceable event kinds. Buzz handles
replacement behavior during ingest. It also has kind-specific validation for
deletions, edits, reactions, admin events, forum votes, and workflow commands.

Side effects are in `crates/buzz-relay/src/handlers/side_effects.rs`. This is
one of the most important files to review because it determines who can add or
remove members, change channel metadata, archive channels, delete groups, and
emit relay-signed discovery or notification events.

Good behavior in this area:

- Last-owner removal is guarded.
- Private channel admin changes require a member with sufficient role.
- Channel archive and visibility changes are restricted.
- Deletion has author and owner/admin checks.
- Open channel joins are allowed, but private channel joins are not open.

The important policy decision is that open channels are intentionally open to
nonmember writes. That is not a bug, but it must be clear in product and
deployment docs.

## Feature Review

### Relay, Auth, And Membership

The relay is the main enforcement point. It verifies Nostr signatures, checks
auth state, checks relay membership when enabled, checks channel membership, and
enforces per-kind rules.

Attack surface:

- WebSocket clients can send auth, event, req, close, and count messages.
- HTTP clients can use `/events`, `/query`, `/count`, media endpoints, Git
  endpoints, and workflow webhooks.
- Redis pub/sub messages are trusted enough to fan out events between relay
  instances.
- Postgres stores plaintext event content and tags for normal events.
- Typesense stores searchable plaintext for indexed events.

Good things:

- WebSocket frame size is bounded in `crates/buzz-relay/src/connection.rs`.
- There is handler concurrency control and heartbeat cleanup for dead clients.
- Unknown event kinds are rejected.
- Event content size and timestamp drift are bounded.
- Relay membership mode fails startup if required owner or relay key config is
  missing.
- Sensitive global kinds are p-gated or author-gated.

Risky things:

- Development defaults are permissive. `BUZZ_REQUIRE_AUTH_TOKEN=false`,
  `BUZZ_REQUIRE_RELAY_MEMBERSHIP=false`, permissive CORS defaults, and the
  hardcoded development relay key are acceptable for local development but
  dangerous if deployed accidentally.
- The relay has no first-class tenant ID in the schema. The isolation model is
  relay membership plus channel membership, not tenant membership.
- Redis, Postgres, Typesense, and S3 are all inside the trusted service
  boundary. A compromise of any of them exposes meaningful data.

### Channels, Messages, Threads, Reactions, And Deletions

Messages are signed Nostr events scoped to channels with `h` tags. Threads are
represented with NIP-10-style relationships. Replies update materialized thread
counters. Reactions and deletions are kind-specific events.

Attack surface:

- Malformed tags can try to confuse thread, channel, or deletion semantics.
- Message content is plaintext unless the specific event kind uses encryption.
- Open channels allow writes by nonmembers.
- Delete events can be used to hide or remove content if policy checks are
  wrong.

Good things:

- Channel membership is checked before normal private channel writes.
- Delete events require exactly one target tag and then ownership or role
  authorization.
- Reactions have deduplication logic.
- Archived channel checks block writes after archive.
- Thread counter behavior is centralized enough to review.

Risky things:

- Any new message-like kind must remember to use `h` tags and the same
  membership checks.
- Any new reply insertion path must update thread counters.
- Global event kinds carrying accidental `h` tags can surprise channel-style
  filters.

### Search

Search uses Typesense through `crates/buzz-search/src/lib.rs`. The relay indexes
accepted events and uses Typesense for NIP-50-style search. Query results are
post-filtered by the relay.

Attack surface:

- Typesense receives plaintext searchable content for indexed event kinds.
- Search filters must stay aligned with relay authorization rules.
- Search queue pressure can drop indexing work.

Good things:

- Gift wraps, DM visibility records, and author-only records are excluded from
  indexing.
- Search results are post-filtered after Typesense returns them.
- Accessible channel IDs are added to search filters.

Risky things:

- Typesense must be protected like a primary data store because it contains
  sensitive channel text for indexed kinds.
- Dropped indexing work can create security-adjacent confusion: users may think
  absence from search means absence from the system.

### Audit

Audit logging is handled by `crates/buzz-audit/src/*` and queued from the relay.
The audit path records accepted events and selected service actions.

Attack surface:

- Audit integrity depends on Postgres and the relay process.
- Queue send failures or worker failures can create gaps if not monitored.

Good things:

- Audit writes are separated from the main event path with backpressure.
- The audit service uses a hash-chain style design.

Risky things:

- This is not a tamper-proof external audit system. A database administrator or
  a relay host compromise can still affect the audit store.
- The security review should check whether queue and worker failures are
  surfaced strongly enough for production operations.

### Media Upload And Download

Media upload uses Blossom-style signed upload auth. The relay validates the
auth event before accepting the body. Files are validated by content type and
stored by hash in S3-compatible storage. The code is mainly in
`crates/buzz-relay/src/api/media.rs` and `crates/buzz-media/src/*`.

Attack surface:

- Uploaders can attempt oversized files, active content, media parser bugs, or
  content type confusion.
- Download URLs are unauthenticated once the hash URL is known.
- S3 credentials and bucket policy are part of the trusted boundary.

Good things:

- Upload auth is verified before reading the body.
- Upload auth binds the expected SHA-256 hash through the `x` tag.
- Image, video, and generic file validation reject many active content types.
- Generic files are served as attachments with defensive headers.
- The response content type comes from the validated sidecar metadata, not from
  untrusted request headers.
- Range reads are capped and stream from storage.

Risky things:

- `GET /media/...` and `HEAD /media/...` are unauthenticated. Media URLs are
  bearer-by-knowledge. Anyone with the URL can fetch the object.
- Media privacy depends on unguessable hashes and controlled sharing, not on
  per-request auth.
- Production mode depends on correct `X-Auth-Token` and S3 configuration.
- S3 buckets must not be public.

### Desktop Client And Workspaces

Desktop is a Tauri app. It holds the user's signing key in the backend process
and exposes signing commands to the frontend.

Important code:

- `desktop/src-tauri/src/app_state.rs`
- `desktop/src-tauri/src/commands/identity.rs`
- `desktop/src-tauri/src/commands/workspace.rs`
- `desktop/src/features/workspaces/workspaceStorage.ts`
- `desktop/src/features/workspaces/useWorkspaceInit.ts`

Attack surface:

- A compromised frontend or webview can ask the Tauri backend to sign events.
- Today it can also call `get_nsec` and export the user's private key.
- Local files in the app data directory contain identity and managed agent
  secrets.
- Workspace switching changes relay URL and remounts React state, but it does
  not create a separate per-workspace identity by default.

Good things:

- The desktop frontend migration strips old `nsec` values out of localStorage.
- The main user key is stored in `identity.key`, not localStorage.
- `identity.key` writes are atomic.
- On Unix, `identity.key` is written with mode `0600`.
- Corrupt `identity.key` files are quarantined rather than overwritten.
- Workspace switching resets known module-level caches in
  `desktop/src/features/workspaces/useWorkspaceInit.ts`.

Risky things:

- `identity.key` is an unencrypted Nostr private key on disk. It is protected by
  filesystem permissions, not by the OS keychain.
- `desktop/src-tauri/src/commands/identity.rs` exposes `get_nsec`. This means an
  XSS or compromised frontend can exfiltrate the full user private key, not only
  request signatures.
- `apply_workspace` accepts an optional `nsec` over IPC. The current frontend
  passes `undefined`, but the command surface still supports changing the
  signing key.
- Desktop workspaces are mostly relay URL profiles. The desktop identity is
  global unless explicitly replaced through env or IPC.

### Mobile Client

Mobile stores workspaces using Flutter secure storage. The workspace JSON can
include `nsec`, and auth uses that key to sign NIP-42 events.

Important code:

- `mobile/lib/shared/workspace/workspace_storage.dart`
- `mobile/lib/shared/workspace/workspace.dart`
- `mobile/lib/shared/auth/auth_provider.dart`

Attack surface:

- Device compromise or app compromise exposes the mobile Nostr key.
- Workspace JSON contains the key material needed to act as the user.

Good things:

- Mobile uses platform secure storage rather than normal localStorage.
- Auth failures can remove invalid or restricted workspaces.

Risky things:

- Secure storage improves local at-rest protection, but the app still needs the
  plaintext key while running.
- The mobile workspace model can hold different keys per workspace, unlike the
  current desktop default. That difference should be intentional and documented.

### Managed Agents

Managed agents are launched by the desktop app. Each managed agent has its own
Nostr keypair. The desktop stores the agent private key and passes it to the
agent process as environment variables.

Important code:

- `desktop/src-tauri/src/managed_agents/types.rs`
- `desktop/src-tauri/src/managed_agents/storage.rs`
- `desktop/src-tauri/src/managed_agents/runtime.rs`
- `desktop/src-tauri/src/managed_agents/env_vars.rs`
- `desktop/src-tauri/src/commands/agents.rs`
- `crates/buzz-acp/src/*`

The flow is: desktop creates an agent record, generates an agent Nostr keypair,
creates an owner delegation auth tag, stores both in the managed agent JSON
store, and later starts the ACP harness. The harness receives the agent command,
agent args, MCP command, relay URL, agent private key, and optional auth tag in
environment variables. The harness connects to Buzz as the agent, listens for
events it is allowed to respond to, and invokes the configured AI agent process.

Attack surface:

- The agent process can sign as the agent.
- The agent receives `BUZZ_PRIVATE_KEY` and `NOSTR_PRIVATE_KEY`.
- The agent may receive `BUZZ_AUTH_TAG`, which proves owner delegation in
  places that accept NIP-OA.
- User-provided environment variables and commands affect the child process.
- Agent stdout and stderr are written to local log files.

Good things:

- Agents use separate keys from the human owner.
- The owner key is not passed to the managed agent.
- Reserved environment variables are stripped from user-provided env.
- Malformed environment keys are rejected, including keys containing `=`.
- Agent process cleanup uses process groups where supported.
- Logs are rotated.

Risky things:

- `managed-agents.json` contains agent private keys and auth tags in app data.
  The write path is atomic, but it does not set explicit `0600` permissions the
  way `identity.key` does.
- Any tool available to the agent can read its own private key from the
  environment unless the tool is deliberately isolated.
- `BUZZ_AUTH_TAG` is a delegated credential. If an attacker has both the agent
  key and auth tag, they can act as that delegated agent wherever the relay
  accepts the owner delegation.
- ACP permission defaults should be reviewed carefully. The ACP config supports
  powerful modes, including bypass-style permissions, and the managed agent
  posture should match the user's expectation for local command execution.

### Developer MCP Server

The developer MCP server exposes local shell and file tools to agents. This is
not a sandbox. It is a controlled way to give an agent access to the developer's
machine.

Important code:

- `crates/buzz-dev-mcp/src/shell.rs`
- `crates/buzz-dev-mcp/src/paths.rs`
- `crates/buzz-dev-mcp/src/shim.rs`

Attack surface:

- The shell tool runs arbitrary `bash -c` commands.
- File tools can read and edit paths outside the repo.
- The shell child environment intentionally includes `BUZZ_PRIVATE_KEY` so the
  `buzz` CLI works.
- Git signing helpers are configured with a temporary keyfile.

Good things:

- Command duration, output size, and artifact size are bounded.
- Process groups are killed on timeout or cancellation where supported.
- The shim removes `NOSTR_PRIVATE_KEY` and uses a `0600` temporary keyfile for
  Git helpers.
- The session temp directory is created with restrictive permissions.

Risky things:

- This is local code execution by design. Any agent with this MCP server can
  read files, run commands, and exfiltrate secrets available to its process.
- There is no path containment in the file tools.
- `BUZZ_PRIVATE_KEY` remains available to shell commands.

### Workflows

Workflows are YAML or JSON definitions executed by the workflow engine. They can
run conditions, add reactions, call webhooks, and wait for approvals.

Important code:

- `crates/buzz-workflow/src/executor.rs`
- `crates/buzz-relay/src/handlers/command_executor.rs`
- `crates/buzz-relay/src/webhook_secret.rs`
- `crates/buzz-relay/src/api/bridge.rs`
- `crates/buzz-db/src/workflow.rs`

Attack surface:

- Workflow definitions can call outbound webhooks.
- Webhook triggers expose an unauthenticated HTTP endpoint protected by a
  workflow secret.
- Conditions are evaluated from user-controlled expressions.
- Approval tokens control resumable workflow execution.

Good things:

- Webhook outbound calls reject private and reserved IPs after DNS resolution.
- The HTTP client pins the resolved IP and disables redirects.
- Webhook response bodies are capped.
- Condition expression length is bounded and evaluation has a timeout.
- Workflow webhook secrets are generated with random UUID v4 values and compared
  in constant time.
- Workflow approval tokens are stored as SHA-256 hashes.

Risky things:

- Workflow webhook secrets are stored inside the workflow definition in
  Postgres under `_webhook_secret`. They are stripped from normal responses and
  returned once at creation, but DB compromise exposes them.
- The webhook endpoint also accepts `?secret=...`. Headers are preferred, but
  query parameters are often logged by proxies.
- Workflow outbound webhooks can exfiltrate data by design. That means workflow
  creation and editing permissions matter.
- The command executor notes that command event idempotency and domain mutation
  writes are not always in a single transaction. That can produce event/mutation
  divergence in failure cases.

### Git

Buzz implements Git smart HTTP backed by object storage and Nostr auth. All Git
HTTP routes require NIP-98 auth. Push policy is enforced by a pre-receive hook
callback.

Important code:

- `crates/buzz-relay/src/api/git/transport.rs`
- `crates/buzz-relay/src/api/git/policy.rs`
- `crates/buzz-relay/src/api/git/store.rs`
- `crates/buzz-relay/src/api/git/cas_publish.rs`
- `crates/git-credential-nostr/src/lib.rs`
- `crates/git-sign-nostr/src/lib.rs`

Attack surface:

- Git clients stream packfiles to the relay.
- The Git credential helper signs NIP-98 credentials.
- Push policy depends on hook callback integrity.
- Object storage holds repository data.
- Git subprocesses are launched by the relay.

Good things:

- Repo IDs are validated: owner must be a lowercase 64-char pubkey, and repo
  names are limited.
- Git subprocess environments are cleared and rebuilt with minimal variables.
- The push policy hook uses HMAC-SHA256 over a canonical payload.
- Hook callbacks have a short freshness window.
- Push policy checks repo ownership, channel binding, archive state, roles, and
  branch rules.
- Object publication uses content-addressed storage and compare-and-swap style
  manifest pointers.

Risky things:

- Git auth intentionally signs the repo-root URL, not the exact Git service URL,
  because Git credential protocol does not pass query strings cleanly.
- Method checking is intentionally relaxed because the same credential is reused
  across GET and POST.
- Packfile bodies are not covered by a NIP-98 payload hash because they are
  streamed.
- Git auth does not use the generic NIP-98 replay cache. The design relies on
  short timestamp freshness, URL binding, TLS, and push policy.
- A captured Git HTTP credential can likely be replayed briefly for the same
  repo within the freshness window.
- Local repo name reservation currently has a cross-instance caveat. Separate
  relay instances with separate disks can race to grant the same display name.

### Pairing

Pairing lets one device transfer secret material to another device using a QR
payload, ephemeral keys, encrypted events, and a short authentication string.

Important code:

- `crates/buzz-core/src/pairing/crypto.rs`
- `crates/buzz-core/src/pairing/qr.rs`
- `crates/buzz-core/src/pairing/session.rs`
- `crates/buzz-pair-relay/src/lib.rs`
- `crates/buzz-pairing-cli/src/main.rs`

Attack surface:

- The QR URI contains a 32-byte session secret.
- Pairing events are encrypted, but the sidecar relay can see metadata.
- Device users must compare the short authentication string.
- Pairing payloads can contain private key material.

Good things:

- Pairing uses ephemeral keys and HKDF-derived session state.
- Transcript hashes are checked.
- Short authentication string confirmation is required before payload acceptance.
- Pairing session timeout is short.
- Pairing event signatures and IDs are verified.
- Pairing relay has tight limits: short TTL, small frames, small per-connection
  budgets, deduplication, and no persistence.
- Secret buffers use zeroization in several places.

Risky things:

- The QR code is a secret. A screenshot, camera, or screen share can expose the
  session secret.
- SAS confirmation is a human security step. If users skip or misunderstand it,
  pairing can be attacked.
- Some temporary plaintext copies can still exist because serde and NIP-44
  internals allocate strings.
- The pairing relay is intentionally unauthenticated and should be exposed only
  in the intended narrow deployment shape.

### Huddle Audio

Huddle audio is a WebSocket audio relay for channel rooms.

Important code:

- `crates/buzz-relay/src/audio/handler.rs`
- `crates/buzz-relay/src/audio/room.rs`
- `crates/buzz-relay/src/audio/wire.rs`

Attack surface:

- Audio clients authenticate over WebSocket.
- The relay receives audio frames and forwards them to room peers.
- Telemetry such as audio level is client-provided.

Good things:

- Huddle connections use NIP-42 auth.
- Relay membership and channel membership are checked.
- Room size, frame size, text control frames, heartbeat, and peer admission are
  bounded.
- Room version is pinned.
- Audio level telemetry is treated as untrusted and clamped.

Risky things:

- Audio is relayed through the server. It is not end-to-end encrypted at the
  application layer.
- The relay can observe room metadata and audio frame traffic.
- Any future moderation or trust decision must not rely on client-supplied audio
  level telemetry.

### NIP-28 Proxy Compatibility

The proxy provides compatibility with clients that expect NIP-28 behavior. It
maps NIP-28-style interactions to Buzz channel behavior.

Important code:

- `crates/buzz-proxy/src/*`
- `NOSTR.md`

Attack surface:

- The proxy has its own server key, salt, admin secret, and API token config.
- It creates deterministic shadow identities for compatibility behavior.
- Private channels require the proxy server pubkey to be a member.

Good things:

- Proxy config keeps the compatibility layer separated from the core direct
  relay path.
- Private channel behavior depends on explicit proxy membership.

Risky things:

- Shadow identity derivation and proxy secrets are security-sensitive. If the
  salt or proxy server key changes, identity continuity and trust assumptions
  change.
- Proxy admin secrets and API tokens need production-grade secret storage.

### Multi-Tenancy And Workspaces

Today, Buzz has workspace switching in the desktop app and relay-level
membership in the relay. It does not yet have a first-class tenant ID that is
carried through every table, query, cache, search index, S3 key, and pub/sub
topic.

Important code:

- `migrations/0001_initial_schema.sql`
- `crates/buzz-db/src/relay_members.rs`
- `crates/buzz-relay/src/handlers/relay_admin.rs`
- `desktop/src/features/workspaces/useWorkspaces.tsx`
- `desktop/src/features/workspaces/workspaceStorage.ts`
- `desktop/src-tauri/src/commands/workspace.rs`

Current behavior:

- Relay membership is global to a relay.
- Channel membership is per channel.
- Events have optional `channel_id`, but no `tenant_id`.
- Global events are shared within the relay.
- Desktop workspaces are mostly relay URL profiles with local metadata.
- Desktop switches workspace by changing relay URL state and remounting the
  React app subtree.

Attack surface:

- Any shared relay deployment uses one database realm unless tenant scoping is
  added.
- Global events, relay members, API tokens, workflows, media, search, audit,
  Redis pub/sub, and S3 keys all need a tenant story before multi-tenant hosting
  can be considered isolated.
- Client-side workspace switching can accidentally retain module-level caches
  unless each singleton is reset.

Good things:

- Workspace switching already has an explicit reset path in
  `desktop/src/features/workspaces/useWorkspaceInit.ts`.
- Relay membership mode provides a useful coarse admission control for private
  deployments.
- Channel access checks are centralized enough that tenant scoping can be added
  deliberately.

Risky things:

- Without a tenant ID, "multi-tenant" means multiple users or workspaces inside
  one relay security domain, not hard tenant isolation.
- Typesense indexes, Redis channels, S3 object keys, audit logs, API tokens, and
  workflow state must all be scoped if a single relay serves multiple tenants.
- Desktop uses one default identity across workspaces. That may be correct for a
  user moving between relays, but it is not the same as per-tenant identity
  separation.

## Keys And Secrets

This section lists the important keys and secrets, where they live, and what
happens if they are compromised.

| Key or secret | Where it is stored or passed | Used for | Compromise impact |
| --- | --- | --- | --- |
| Human Nostr private key, desktop | `{app_data_dir}/identity.key`; `BUZZ_PRIVATE_KEY` env can override; exportable through `get_nsec` | Signs user events, NIP-42 auth, NIP-98 auth, observer control events | Full account compromise for that Nostr identity |
| Human Nostr private key, mobile | Flutter secure storage workspace JSON, including `nsec` | Signs mobile user events and auth | Full account compromise on mobile identity |
| Human Nostr private key, CLI and Git tools | `BUZZ_PRIVATE_KEY`, `NOSTR_PRIVATE_KEY`, or Git config `nostr.keyfile` | CLI event signing, Git credentials, Git object signatures | Full account compromise for that key |
| Managed agent private key | `desktop` app data under `agents/managed-agents.json`; child env as `BUZZ_PRIVATE_KEY` and `NOSTR_PRIVATE_KEY`; dev MCP temp keyfile for Git | Signs agent events and auth | Attacker can act as the managed agent |
| NIP-OA auth tag | Managed agent record and child env `BUZZ_AUTH_TAG`; also Git config path in some tools | Proves an agent is delegated by an owner key | With the agent key, attacker can act as delegated agent where NIP-OA is trusted |
| Relay private key | `BUZZ_RELAY_PRIVATE_KEY` env; hardcoded dev fallback only when auth token is not required | Signs relay/system/discovery/notification events | Attacker can forge relay-signed system behavior and damage trust in relay events |
| Relay owner pubkey | `RELAY_OWNER_PUBKEY` env and `relay_members` table | Bootstraps relay owner membership | Wrong value gives owner control to wrong pubkey |
| API tokens | Raw token returned to caller; SHA-256 hash stored in `api_tokens` | Scoped API access, especially media upload in production | Raw token grants its configured scopes until revoked or expired |
| Workflow webhook secret | Stored inside workflow definition as `_webhook_secret`; returned once at creation; sent as `X-Webhook-Secret` or `?secret=` | Authenticates external webhook triggers | Anyone with the secret can trigger that workflow |
| Workflow approval token | Raw token generated by workflow engine; SHA-256 hash stored in DB | Approves or denies waiting workflow steps | Hash leak is less useful than raw token, but approval authorization still needs review around token delivery |
| Blossom upload auth event | Short-lived signed event in HTTP `Authorization: Nostr ...` | Authorizes one upload hash | Replay within validity window may upload the same hash if other checks allow it |
| S3 media credentials | `BUZZ_S3_ACCESS_KEY`, `BUZZ_S3_SECRET_KEY`, `BUZZ_S3_ENDPOINT`, `BUZZ_S3_BUCKET` | Media object storage | Attacker can read or write stored media if bucket policy also allows it |
| S3 Git credentials | `BUZZ_GIT_S3_*` or fallback S3 env vars | Git object and manifest storage | Attacker can read or tamper with Git backing storage depending on bucket permissions |
| Typesense API key | `TYPESENSE_API_KEY` env | Search indexing and querying | Attacker can read indexed plaintext content or tamper with search results |
| Database URL and credentials | `DATABASE_URL` env | Primary Postgres access | Attacker can read and mutate almost all application state |
| Redis URL and credentials | `REDIS_URL` env | Pub/sub, presence, cache coordination | Attacker can inject or observe live coordination data depending on Redis access |
| Git hook HMAC secret | `BUZZ_GIT_HOOK_HMAC_SECRET` env, random dev fallback | Authenticates pre-receive hook callbacks to relay policy endpoint | Attacker can forge hook policy callbacks |
| Proxy server key, salt, admin secret, API token | Proxy env vars documented in `NOSTR.md` | NIP-28 compatibility behavior and proxy administration | Attacker can impersonate proxy behavior or administer proxy depending on secret |
| Pairing session secret | Encoded in `nostrpair://...secret=...` QR URI | Establishes pairing session | Anyone who sees it can participate in the pairing attempt unless SAS confirmation stops them |
| Pairing ephemeral private keys | In memory during a pairing session | Establishes encrypted pairing messages and SAS | Compromise during the short session can attack that pairing |
| NIP-44 derived conversation keys | Derived in memory from Nostr private keys | Encrypts and decrypts NIP-44 payloads | Compromise allows plaintext access for that derived conversation context |

## Security Strengths

These are the design and implementation choices that look good from a security
review point of view.

- Normal actions are signed by user or agent Nostr keys.
- Event IDs and signatures are verified before storage.
- Unknown kinds are rejected.
- NIP-42 auth uses random challenges, relay binding, and timestamp freshness.
- NIP-98 auth verifies URL, method, timestamp, signature, and payload hash when
  present.
- WebSocket frames, event content, historical limits, media sizes, workflow
  expressions, webhook responses, and audio frames have bounds.
- Channel and global subscriptions are kept separate.
- Private channel live fanout is filtered again at send time.
- Sensitive global kinds are p-gated or author-gated.
- Search results are post-filtered after Typesense.
- Gift wraps and other sensitive global kinds are excluded from search.
- Relay membership mode fails startup if required production secrets are absent.
- Desktop no longer stores the user `nsec` in localStorage.
- Desktop user key writes use atomic writes and `0600` permissions on Unix.
- Mobile uses secure storage for workspace secrets.
- Managed agent environment handling strips reserved variables and rejects
  malformed env keys.
- The dev MCP shim removes `NOSTR_PRIVATE_KEY` and uses a `0600` temporary
  keyfile for Git helpers.
- Workflow outbound webhooks have SSRF defenses: private IP rejection, DNS
  pinning, no redirects, timeout, and response body cap.
- Git subprocess environments are hardened.
- Git push policy callbacks use HMAC and a short freshness window.
- Pairing uses ephemeral keys, transcript binding, SAS confirmation, and a
  tightly limited sidecar relay.
- Huddle audio has auth, membership checks, frame bounds, room caps, and
  untrusted telemetry handling.

## Security Sharp Edges

These are the areas I would prioritize in a security review.

1. Desktop private key exposure

   `desktop/src-tauri/src/commands/identity.rs` exposes `get_nsec`, and the
   desktop key is stored unencrypted in `identity.key`. Filesystem permissions
   help, but a frontend compromise can export the key directly. For a desktop
   app that renders remote or semi-trusted content, this is one of the highest
   risk areas.

2. Managed agent private key storage

   Managed agent records contain agent private keys and auth tags. They are
   stored in JSON in the app data directory and passed to child processes in env
   vars. This is simple and useful, but it means any local process compromise or
   agent tool compromise can steal an agent identity.

3. Agent tool authority

   `buzz-dev-mcp` is intentionally powerful. It can run shell commands and
   access files outside the repo. This should be described to users as local
   code execution, not as a restricted tool sandbox.

4. Multi-tenancy is not hard isolation yet

   The schema does not carry a tenant ID. Relay membership and channel
   membership are useful, but they are not the same thing as tenant isolation.
   Before hosting multiple tenants on one relay, tenant scope needs to be added
   to Postgres queries, Typesense indexes, Redis pub/sub, S3 object keys, audit
   logs, workflow state, API tokens, media, Git storage, caches, and relay admin
   state.

5. Development defaults are unsafe for production

   The relay defaults are convenient for local development. In production, the
   dangerous settings are no relay membership, no auth token requirement,
   permissive CORS, dev Typesense and S3 credentials, and the hardcoded dev
   relay key fallback.

6. Git NIP-98 replay tradeoffs

   Git auth intentionally relaxes URL, method, body hash, and replay behavior
   because of Git smart HTTP constraints. The design may be acceptable, but it
   should be explicitly reviewed as a replay window protected by TLS and short
   freshness, not as full NIP-98 request binding.

7. NIP-98 payload hash is optional

   The generic NIP-98 verifier accepts missing payload hashes. For non-streaming
   state-changing routes, Buzz should consider requiring payload binding unless
   there is a specific reason not to.

8. Media URLs are bearer-by-knowledge

   Media download is unauthenticated. That is a valid product choice if media
   hashes are treated as share links, but it should not be mistaken for private
   media authorization.

9. Workflow secrets are stored in database definitions

   Workflow webhook secrets are stripped from normal responses, but they are in
   Postgres. Query-param secrets are also supported. Header-only secrets and
   stronger secret storage would reduce accidental leakage.

10. Typesense contains sensitive plaintext

   Typesense should be treated as sensitive infrastructure. It can contain
   indexed channel content even when Postgres access is otherwise controlled.

11. Global event `h` tag behavior is surprising

   Global-only events can still contain signed `h` tags. Because filter matching
   respects explicit tags, these events can match channel-style filters even
   though they are stored globally. That is not necessarily a data leak, but it
   is a confusing edge that can affect automation.

12. Relay/system key blast radius

   The relay private key signs system events, discovery events, and membership
   notifications. A relay key compromise lets an attacker forge relay-authored
   system behavior. It should be stored and rotated like a production signing
   key, not like a normal config value.

## Suggested Review Priorities

For a first security review pass, I would focus on these areas in order.

1. Desktop key handling and Tauri command exposure

   Review `get_nsec`, `import_identity`, `sign_event`, observer encryption
   commands, frontend trust assumptions, CSP, remote content rendering, and
   whether the private key can move to OS keychain-backed signing without direct
   export to the frontend.

2. Managed agent launch and local tool model

   Review managed agent key storage permissions, auth tag storage, env passing,
   logs, command configuration, ACP permission defaults, and how clearly the UI
   communicates local command authority.

3. Relay authorization invariants

   Review `ingest.rs`, `req.rs`, `event.rs`, `side_effects.rs`, and
   `filter.rs` together. The key question is whether every event kind has clear
   rules for who can write it, who can read it, whether it is channel or global,
   whether search can index it, and whether live fanout matches historical
   query behavior.

4. Multi-tenant design

   Decide whether a "tenant" is a relay, an org inside a relay, a workspace, or
   something else. If tenants share one relay process and database, add tenant
   scope explicitly before treating the deployment as isolated.

5. Git auth and push policy

   Review the Git NIP-98 exception model, replay window, credential helper
   behavior, hook HMAC, object storage CAS behavior, and repo name reservation
   across multiple relay instances.

6. Workflow and media exfiltration

   Review who can create workflows, where webhook secrets appear, whether query
   secrets can be removed, what data workflows can send out, and whether media
   URL privacy matches product expectations.

7. Production configuration hardening

   Add deployment checks or docs that make production requirements hard to miss:
   relay private key, relay membership, strict CORS, real Typesense key, private
   S3 bucket, strong Git hook secret, TLS, Redis/Postgres network isolation, and
   secret rotation paths.

## Things To Know While Reviewing Code

- The most important relay files are `ingest.rs`, `event.rs`, `req.rs`,
  `side_effects.rs`, and `filter.rs`. Read them as a set.
- Do not assume a Nostr kind is safe because it is signed. Signatures prove who
  authored the event; they do not say the author was allowed to perform the
  action.
- Do not assume global events are harmless. Global events need their own read
  gates and indexing rules.
- Do not assume desktop workspaces are tenants. They are local relay profiles
  unless backed by server-side tenant isolation.
- Do not assume agents are sandboxed. The default developer tool model gives
  agents local machine power.
- Do not assume encrypted content hides metadata. Gift wraps, engrams,
  reminders, observer frames, and pairing events still expose kinds, pubkeys,
  timestamps, tags, and routing metadata.
- Do not assume relay-signed events are user intent. They are relay assertions,
  and their trust depends on the relay private key and relay implementation.
- Do not assume media hashes are private authorization. They are unguessable
  object references, but anyone who learns the URL can fetch the object.
