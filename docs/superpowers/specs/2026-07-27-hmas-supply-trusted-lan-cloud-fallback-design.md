# HMAS Supply Trusted-LAN Sources and Automatic Cloud Fallback

## Purpose

Make the accepted Phase 4 macOS Command Console usable with the existing home
Memory and RAG services without completing the deferred signed-mirror and
replication programme.

The owner treats the closed home LAN and VPN-only remote access as the approved
perimeter for `OFFICIAL` material. The existing RAG corpus already contains
Defence publications at that level. The owner also explicitly accepts sending
bounded `OFFICIAL` evidence to configured cloud models when local inference
fails.

This is a focused commissioning feature. It does not revive Phase 5 workspace
actions, RAG 2.0, bidirectional Memory replication, immutable RAG snapshots, or
the wider Phase 6 assurance ecosystem.

## Selected Architecture

Buzz adds an explicit `OFFICIAL_TRUSTED_LAN` source mode. The existing strict
literal-loopback, authenticated, signed-snapshot mode remains available and is
not weakened.

In trusted-LAN mode, the Tauri process owns an embedded compatibility gateway:

- the gateway binds only to a random literal-loopback port;
- it uses independent, random per-launch credentials for app and model access;
- its only upstreams are the configured fixed private IPv4 Memory and RAG MCP
  endpoints;
- it disables environment proxies, refuses redirects, uses bounded requests
  and responses, and exposes only the fixed tools needed by the brief;
- it translates existing service responses into an explicit
  `trusted-lan-observed` evidence contract without claiming source signatures,
  immutable revisions, or a signed RAG snapshot.

The initial upstreams are:

- Memory: `http://192.168.1.26:8006/mcp`
- RAG: `http://192.168.1.107:8005/mcp/`

The protected configuration accepts literal RFC1918 IPv4 HTTP endpoints only.
It rejects DNS names, user information, queries, fragments, redirects, public
addresses, loopback upstreams, unspecified addresses, and unexpected paths or
tools. Configuration changes remain outside renderer control.

## Evidence and Provenance

RAG evidence retains:

- query;
- point ID;
- document ID and name;
- collection;
- page number and section path when present;
- retrieval timestamp; and
- the quoted passage.

Memory evidence retains:

- event or entity identity;
- event and recording timestamps;
- named entities and tags when present;
- retrieval timestamp; and
- bounded quoted content.

Every record is marked `trusted-lan-observed`. The UI and persisted brief state
must never describe this evidence as cryptographically signed, locally
mirrored, immutable, or conflict-free.

At run start Buzz calculates an informational SHA-256 fingerprint over the
canonical RAG collection names and chunk counts. It recalculates the
fingerprint at run completion. A difference adds a non-blocking
limitation stating that the catalogue changed during generation. It never
restarts, invalidates, or fails the brief. Individual result citations are the
operative provenance.

Memory, RAG, Calendar, Reminders, Notes, and files fail independently. An
unavailable or malformed source degrades the affected sections and creates an
explicit limitation; it does not block the whole brief. A brief requires an
unlocked Buzz identity and at least one usable model route, not every source.

## Model Routing and Cloud Egress

The fixed automatic route is:

1. MacBook LM Studio;
2. the configured home LiteLLM service; and
3. direct OpenAI Responses.

An eligible local failure automatically advances to the next configured route
without another prompt. Eligible failures are model unavailable, transport
failure, timeout, provider rejection, or invalid structured model output.
Cancellation, invalid application configuration, malformed trusted state, and
policy-integrity failures stop immediately and do not trigger cloud fallback.

Cloud specialists receive only the bounded source ledger already collected by
Buzz. They receive no LAN addresses, MCP credentials, local tools, unrestricted
corpus access, hidden model reasoning, or provider credentials. The Chief of
Staff receives only validated specialist contributions and the bounded ledger.

Each provider attempt is non-retrying. A fallback is a new attempt with a new
provider after the prior attempt reaches a terminal failure. The persisted
audit records:

- adviser;
- provider and model;
- start and completion time;
- outcome;
- stable fallback reason;
- hashes of the transmitted source IDs; and
- whether the provider was local, trusted-LAN, or cloud.

The brief remains classified `OFFICIAL`. The Command Console displays
`OFFICIAL - TRUSTED LAN`, `Local preferred - Automatic cloud fallback`, and the
actual provider used for each contribution.

Provider endpoints, model IDs, and Keychain credential references live in
protected native configuration. Renderer input, retrieved documents, personas,
and model output cannot add a provider, change an endpoint, or disable the
audit. LiteLLM remains preferred over direct OpenAI when both are available.

## Failure Behaviour

- One failed source produces a degraded section, not a failed run.
- One failed specialist may fall through the configured model chain.
- Exhausting the model chain fails that specialist visibly while other
  specialists complete.
- The Chief of Staff consolidates completed contributions and preserves missing
  adviser, limitation, and dissent information.
- Cancellation prevents further source calls and provider attempts.
- Oversized or malformed source responses are discarded and reported without
  including their bodies in diagnostics.
- Endpoint, provider, and credential errors use stable redacted codes.
- Secrets, source passages, prompts, model reasoning, and provider response
  bodies are excluded from logs and lifecycle status events.

## User Experience

The System Status area adds a trusted-LAN summary for Memory and RAG:

- configured endpoint identity;
- reachability;
- observed tools;
- collection or event availability;
- last successful observation; and
- `unsigned trusted-LAN evidence` assurance.

The Daily Command Brief shows:

- `OFFICIAL - TRUSTED LAN`;
- local-first automatic cloud fallback;
- provider/model per adviser;
- fallback reasons;
- source availability and freshness;
- catalogue-change warning when observed; and
- the existing advisory, non-accredited limitation.

There is no per-run approval dialog. The protected configuration is the durable
owner acknowledgement for automatic cloud fallback.

## Test and Acceptance

Test-driven implementation must prove:

- only protected literal-RFC1918 endpoint configurations are accepted;
- redirects, proxies, DNS names, public hosts, wrong paths, excess tools, and
  oversized responses are rejected;
- the compatibility gateway is loopback-only and requires its per-launch
  credentials;
- real legacy Memory and RAG response shapes become bounded
  `trusted-lan-observed` evidence with exact citations;
- no synthetic signature, revision, or immutable-snapshot claim is emitted;
- one unavailable source degrades rather than blocks a brief;
- catalogue fingerprint changes warn but never restart or fail a run;
- routing order is LM Studio, LiteLLM, then OpenAI;
- cancellation and policy-integrity errors never trigger fallback;
- cloud requests contain bounded evidence but no LAN endpoints, MCP tools, or
  credentials;
- every provider attempt and fallback reason appears in persisted audit data;
- the Chief of Staff remains tool-free and cannot invent unsupported findings;
  and
- existing strict Phase 4 behaviour remains green when trusted-LAN mode is not
  configured.

Live acceptance uses the current home Memory and RAG endpoints, the loaded
LM Studio model, the configured LiteLLM service, and direct OpenAI only if the
earlier routes fail. It generates one real Daily Command Brief, inspects cited
RAG and Memory evidence, confirms the provider audit, and verifies the
catalogue-change path is warning-only.

## Explicitly Deferred

- Signed local RAG snapshots and offline corpus mirroring.
- Conflict-aware Memory replication and a MacBook-local writable authority.
- RAG 2.0 backup, restore, rollback, and golden-query equivalence.
- Phase 5 workspace actions.
- Accreditation or treatment as an operational/navigation decision system.
