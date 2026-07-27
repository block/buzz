# HMAS Supply Trusted-LAN and Cloud Fallback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generate a real Phase 4 Daily Command Brief from the existing LAN Memory/RAG services, preferring LM Studio and automatically falling through LiteLLM to OpenAI.

**Architecture:** Add a protected trusted-LAN configuration and an embedded loopback MCP compatibility gateway owned by the Tauri process. Translate legacy LAN evidence into an explicitly weaker observed-evidence contract, then route each bounded adviser request through a native local-first provider chain with persisted route audit.

**Tech Stack:** Rust, Tauri 2, Axum, Reqwest, LM Studio native API, OpenAI-compatible Chat Completions, OpenAI Responses, React 19, Node test runner.

## Global Constraints

- Keep the accepted strict signed-snapshot mode unchanged when trusted-LAN mode is absent.
- Trusted-LAN upstreams must be literal RFC1918 IPv4 HTTP URLs with exact Memory `/mcp` and RAG `/mcp/` paths.
- Disable environment proxies and redirects for LAN and cloud clients.
- Cloud fallback order is LM Studio, LiteLLM, OpenAI and requires no per-run approval.
- Cloud requests may contain bounded OFFICIAL evidence but never LAN endpoints, MCP credentials, local tools, hidden reasoning, or unrestricted corpus access.
- RAG catalogue changes are warning-only and never restart, invalidate, or fail a brief.
- Source failures degrade independently; cancellation and policy-integrity failures never trigger model fallback.
- New readable text uses existing rem-based Tailwind tokens.
- No new `unsafe`, production `unwrap()`, or production `expect()`.

---

### Task 1: Protected trusted-LAN and cloud route configuration

**Files:**
- Create: `desktop/src-tauri/src/command_services/trusted_lan.rs`
- Create: `desktop/src-tauri/src/command_services/trusted_lan_tests.rs`
- Modify: `desktop/src-tauri/src/command_services/mod.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Create: `desktop/src-tauri/trusted-lan-sources.example.json`

**Interfaces:**
- Produces: `TrustedLanConfig::load(&Path, &SecretStore) -> Result<TrustedLanConfig, TrustedLanError>`
- Produces: `TrustedLanEndpoint::parse_memory(&str)` and `TrustedLanEndpoint::parse_rag(&str)`
- Produces: `CloudRouteConfig { litellm, openai }` with Keychain references and exact model IDs.

- [ ] **Step 1: Write endpoint and protected-file tests**

Add table-driven tests whose literal expected results prove that only URLs such
as these are accepted:

```rust
assert!(TrustedLanEndpoint::parse_memory(
    "http://192.168.1.26:8006/mcp"
).is_ok());
assert!(TrustedLanEndpoint::parse_rag(
    "http://192.168.1.107:8005/mcp/"
).is_ok());
for rejected in [
    "https://192.168.1.26:8006/mcp",
    "http://memory.home.arpa:8006/mcp",
    "http://127.0.0.1:8006/mcp",
    "http://8.8.8.8:8006/mcp",
    "http://192.168.1.26:8006/mcp?x=1",
] {
    assert!(TrustedLanEndpoint::parse_memory(rejected).is_err());
}
```

Name the production break: accepting a non-private, ambiguous, redirected, or
renderer-controlled source route.

- [ ] **Step 2: Run RED tests**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml trusted_lan_tests -- --nocapture
```

Expected: compile failure because `trusted_lan` and its types do not exist.

- [ ] **Step 3: Implement the minimal closed config**

Implement strict deserialization with `deny_unknown_fields`, mode
`OFFICIAL_TRUSTED_LAN`, exact tool sets, non-empty model IDs, Keychain
credential references, a durable `automatic_cloud_fallback_acknowledged: true`
field, and protected-file loading through the existing `ProtectedFile`.

- [ ] **Step 4: Run GREEN tests and formatting**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml trusted_lan_tests -- --nocapture
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml -- --check
```

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src/command_services desktop/src-tauri/src/lib.rs \
  desktop/src-tauri/trusted-lan-sources.example.json
git commit -m "feat(command): validate trusted LAN routes"
```

### Task 2: Embedded compatibility gateway and observed evidence

**Files:**
- Create: `desktop/src-tauri/src/command_services/trusted_lan/gateway.rs`
- Create: `desktop/src-tauri/src/command_services/trusted_lan/legacy_mcp.rs`
- Create: `desktop/src-tauri/src/command_services/trusted_lan/evidence.rs`
- Create: `desktop/src-tauri/src/command_services/trusted_lan_tests/gateway.rs`
- Create: `desktop/src-tauri/src/command_services/trusted_lan_tests/evidence.rs`
- Modify: `desktop/src-tauri/src/command_services/trusted_lan.rs`
- Modify: `desktop/src-tauri/src/command_services/policy/catalog.rs`
- Modify: `desktop/src-tauri/src/command_brief/sources.rs`
- Modify: `desktop/src-tauri/src/command_brief/types.rs`

**Interfaces:**
- Produces: `TrustedLanGateway::start(config) -> Result<TrustedLanGateway, TrustedLanError>`
- Produces: `TrustedLanGateway::runtime_catalog() -> AdviserRuntimeCatalog`
- Produces: `TrustedLanSourceBackend` implementing `SourceBackend`
- Produces: `ObservedRagEvidence` and `ObservedMemoryEvidence` with assurance `trusted-lan-observed`.

- [ ] **Step 1: Write failing gateway boundary tests**

Use real loopback fake upstreams and assert observable network behaviour:

```rust
let gateway = TrustedLanGateway::start_for_test(config, upstreams).await?;
assert_eq!(gateway.local_addr().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
assert_eq!(unauthenticated_tools_list(gateway.endpoint()).await, StatusCode::UNAUTHORIZED);
assert_eq!(
    authenticated_tool_names(&gateway).await?,
    ["command_memory_context", "list_collections", "search_knowledge_base"]
);
```

Also prove redirects are refused, environment proxy variables are ignored,
responses over the byte bound fail, and shutdown closes the listener.

- [ ] **Step 2: Run RED gateway tests**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml \
  command_services::trusted_lan_tests::gateway -- --nocapture
```

Expected: compile failure because the gateway is absent.

- [ ] **Step 3: Implement the bounded loopback gateway**

Use Axum on `TcpListener::bind((Ipv4Addr::LOCALHOST, 0))`, random independent
per-launch bearer/attestation values, exact JSON-RPC initialize/tools
list/tools call handling, and an upstream Reqwest client with:

```rust
Client::builder()
    .no_proxy()
    .redirect(reqwest::redirect::Policy::none())
    .timeout(Duration::from_secs(10))
    .build()
```

Forward only the fixed legacy tools. Do not proxy arbitrary JSON-RPC methods.

- [ ] **Step 4: Write failing observed-evidence tests**

Feed complete real legacy response fixtures and assert hand-derived fields:

```rust
assert_eq!(rag.assurance(), SourceAssurance::TrustedLanObserved);
assert_eq!(rag.source_id(), "cbc50f57-f36b-57f3-abbb-64f235e8f418");
assert_eq!(rag.page_number(), Some(5));
assert_eq!(memory.event_id(), "01KYGFENY8HP4FBP8F684WQ0WT");
assert!(!serialized.contains("signature"));
assert!(!serialized.contains("immutable"));
```

Add a test where the start and finish catalogue fingerprints differ and assert
the resulting context succeeds with exactly one warning.

- [ ] **Step 5: Implement observed evidence and source backend**

Add `SourceAssurance` to ledger entries, retain Phase 4 parsing of version 1
history, emit version 2 for new briefs, and implement a trusted-LAN backend
that uses event/point identities rather than synthesizing Phase 3 revision or
snapshot claims. Use the informational catalogue fingerprint as the run
`snapshot_id`.

- [ ] **Step 6: Run GREEN source tests**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml trusted_lan -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::sources -- --nocapture
```

- [ ] **Step 7: Commit**

```bash
git add desktop/src-tauri/src/command_services desktop/src-tauri/src/command_brief
git commit -m "feat(command): adapt trusted LAN evidence"
```

### Task 3: Automatic local to LiteLLM to OpenAI routing

**Files:**
- Create: `desktop/src-tauri/src/command_brief/cloud.rs`
- Create: `desktop/src-tauri/src/command_brief/cloud_tests.rs`
- Create: `desktop/src-tauri/src/command_brief/router.rs`
- Create: `desktop/src-tauri/src/command_brief/router_tests.rs`
- Modify: `desktop/src-tauri/src/command_brief/mod.rs`
- Modify: `desktop/src-tauri/src/command_brief/lmstudio.rs`
- Modify: `desktop/src-tauri/src/command_brief/orchestrator.rs`
- Modify: `desktop/src-tauri/src/command_brief/types.rs`

**Interfaces:**
- Produces: `CloudAdviserClient::run_specialist` and `run_chief_of_staff`
- Produces: `AdviserRouter::run_specialist` and `run_chief_of_staff`
- Produces: `AdviserRouteAudit { adviser, attempts, transmitted_source_hashes }`
- Consumes: validated `SpecialistAdviserRequest` and `ChiefOfStaffRequest`; never raw MCP access.

- [ ] **Step 1: Write failing cloud request-redaction tests**

Capture requests at real loopback fake Chat Completions and Responses servers.
Assert that the serialized body contains cited bounded passages and excludes:

```rust
for forbidden in [
    "192.168.1.26",
    "192.168.1.107",
    "mcp-session-id",
    "Authorization: Bearer",
    "command_memory_context",
] {
    assert!(!request_body.contains(forbidden));
}
```

Assert complete provider response fixtures parse through the existing closed
`AdviserContribution` and `ChiefOfStaffConsolidation` contracts.

- [ ] **Step 2: Run RED cloud tests**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml cloud_tests -- --nocapture
```

Expected: compile failure because the focused cloud client is absent.

- [ ] **Step 3: Implement focused cloud clients**

Implement only non-streaming LiteLLM Chat Completions and OpenAI Responses.
Use protected endpoint/model configuration and Keychain tokens. Use the same
fixed personas, evidence prompt, output size bounds, JSON contract parser,
redacted diagnostics, cancellation token, no redirects, and no proxies as the
local executor.

- [ ] **Step 4: Write failing fallback classification tests**

Use three deterministic fake clients:

```rust
assert_eq!(
    route_attempts(local_timeout, litellm_success, openai_unused),
    ["lm_studio:timeout", "litellm:success"]
);
assert_eq!(
    route_attempts(local_transport, litellm_rejected, openai_success),
    ["lm_studio:transport", "litellm:provider_rejected", "openai:success"]
);
assert_eq!(route_attempts(local_cancelled, any, any), ["lm_studio:cancelled"]);
assert_eq!(route_attempts(local_policy_rejected, any, any), ["lm_studio:policy_rejected"]);
```

Name the production break: falling back on cancellation/integrity failure or
using providers out of order.

- [ ] **Step 5: Implement the router and audit**

Map only eligible terminal failures to fallback. Persist provider/model,
timestamps, outcome, stable reason, cloud/local flag, and SHA-256 hashes of
transmitted ledger IDs. Never persist provider bodies, prompts, passages,
credentials, or reasoning in route audit.

- [ ] **Step 6: Run GREEN routing tests**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::cloud -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::router -- --nocapture
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::orchestrator -- --nocapture
```

- [ ] **Step 7: Commit**

```bash
git add desktop/src-tauri/src/command_brief
git commit -m "feat(command): add automatic cloud fallback"
```

### Task 4: Status, persisted wire compatibility, and Command Console UI

**Files:**
- Modify: `desktop/src-tauri/src/command_services/policy/status.rs`
- Modify: `desktop/src-tauri/src/command_brief/wire.rs`
- Modify: `desktop/src-tauri/src/command_brief/audit.rs`
- Modify: `desktop/src/features/command-console/domain/commandBrief.ts`
- Modify: `desktop/src/features/command-console/hooks/useCommandConsoleStatus.ts`
- Modify: `desktop/src/features/command-console/ui/CommandConsoleScreen.tsx`
- Modify: corresponding existing Rust and `.test.mjs` files.

**Interfaces:**
- Produces UI fields `sourceMode`, `sourceAssurance`, `providerAttempts`, and `fallbackReason`.
- Preserves reading version 1 strict Phase 4 history while writing version 2 trusted-LAN briefs.

- [ ] **Step 1: Write failing parser and UI projection tests**

Assert version 1 history still renders unchanged. Add a literal version 2
fixture and assert visible text:

```text
OFFICIAL - TRUSTED LAN
Local preferred - Automatic cloud fallback
Unsigned trusted-LAN evidence
Operations Adviser: LiteLLM (LM Studio timeout)
```

Assert a catalogue change renders a warning and the brief remains completed.

- [ ] **Step 2: Run RED Rust and frontend tests**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief -- --nocapture
cd desktop
node --import ./test-loader.mjs --experimental-strip-types --test \
  src/features/command-console/**/*.test.mjs
```

- [ ] **Step 3: Implement status, wire, and UI changes**

Extend the closed Rust/TypeScript parsers together, retain strict unknown-field
rejection per version, show actual provider attempts without secrets, and keep
the existing advisory/non-accredited warning prominent.

- [ ] **Step 4: Run GREEN UI and focused integration tests**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief -- --nocapture
cd desktop
pnpm test
pnpm run build:e2e
```

- [ ] **Step 5: Commit**

```bash
git add desktop/src-tauri/src desktop/src/features/command-console
git commit -m "feat(command-console): show trusted LAN routing"
```

### Task 5: Commission the current MacBook and generate the first brief

**Files:**
- Create: `docs/command-console/trusted-lan-commissioning.md`
- Create: `scripts/check-trusted-lan-command-brief.sh`
- Create: `scripts/tests/check-trusted-lan-command-brief-test.sh`
- Modify: `justfile`
- External protected file: macOS app configuration `trusted-lan-sources.json`
- External credentials: existing macOS Keychain entries only.

**Interfaces:**
- Produces: `just check-trusted-lan-command-brief`
- Produces: a locally signed Phase 4-derived `Buzz.app` and one persisted real brief.

- [ ] **Step 1: Write the failing executable acceptance test**

The script test must run fake LAN and provider endpoints and prove child
failure propagation, literal endpoint validation, warning-only catalogue
changes, and success-claim suppression when no brief is produced.

- [ ] **Step 2: Run RED acceptance test**

```bash
. ./bin/activate-hermit
bash scripts/tests/check-trusted-lan-command-brief-test.sh
```

Expected: failure because the runner and Just target do not exist.

- [ ] **Step 3: Implement the bounded runner and operator documentation**

The runner prints endpoint identities, model/provider names, source counts,
brief ID, provider attempt summary, and citation counts. It must not print
tokens, prompts, passages, credentials, or response bodies.

- [ ] **Step 4: Run focused and aggregate gates**

```bash
. ./bin/activate-hermit
just check-trusted-lan-command-brief
just check-daily-command-brief
just ci
git diff --check
```

- [ ] **Step 5: Provision without exposing secrets**

Write the protected app config with mode `0600`, reuse existing Keychain
credentials through `SecretStore`, and verify:

```text
Memory 192.168.1.26:8006 reachable
RAG 192.168.1.107:8005 reachable
LM Studio qwen/qwen3.6-27b ready
LiteLLM configured or visibly unavailable
OpenAI configured or visibly unavailable
```

- [ ] **Step 6: Build, locally sign, launch, and generate**

Build the release app with full Xcode, install real external binaries into the
generated bundle, apply an ad-hoc local signature, launch the exact app path,
generate one Daily Command Brief, and inspect its citations, limitations,
provider audit, and encrypted-history reload.

- [ ] **Step 7: Commit, push, and update the draft PR**

```bash
git add docs/command-console scripts justfile
git commit -m "docs(command-console): commission trusted LAN brief"
. ./bin/activate-hermit
git push
```

Record the verified architecture decision, configuration shape, live
endpoints, first-brief result, and operational gotchas in Memory MCP with
`agent: CODEX`.
