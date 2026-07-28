# Command Team Logistics, Intelligence, and Doctrine-Guided Advice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Logistics and Maritime N2 as native Command Team advisers, make adviser conversations doctrine-aware, and add bounded World Monitor intelligence to N2 conversations and the Daily Command Brief.

**Architecture:** Extend the existing managed-agent and command-brief paths rather than creating another application. A small shared Rust library owns MCP HTTP parsing, OAuth token rotation, the curated World Monitor tool vocabulary, freshness handling, and one locked JSON quota/cache ledger used by both the desktop brief collector and `buzz-dev-mcp`. Existing Buzz discussions, encrypted outcomes, RAG, Memory, cloud/local routing, signed brief persistence, and naval UI remain authoritative.

**Tech Stack:** Rust 1.88, Tokio, Reqwest, RMCP, Serde, OAuth 2.1 PKCE, Tauri 2, React 19, TypeScript, Node test runner, Playwright, pnpm, Hermit.

> **28 July OAuth correction:** World Monitor Pro supplies MCP access through
> OAuth and does not issue an API key. Any API-key or
> `command.world-monitor.api-key` detail later in this execution log describes
> the superseded first implementation. The accepted implementation is the
> browser OAuth flow, rotating local credentials, `Connect World Monitor` UI,
> and `COMMAND_ADVISER_WORLD_MONITOR_OAUTH_PATH`.

## Global Constraints

- Activate Hermit before every Cargo, `just`, Git hook, or verification command: `. ./bin/activate-hermit`.
- Follow test-driven development: write one failing behavioural test, run it and confirm the expected failure, then write the minimum production code.
- The standing Command Team contains exactly eight advisers; the Daily Command Brief contains exactly seven specialist contributions plus the Chief of Staff.
- Every adviser seeks the logical RAG collection `ADF Doctrine` before broader knowledge for substantive advice.
- Doctrine is guidance, not a response gate. No result or unavailable RAG must not cause an adviser to refuse an assessment.
- World Monitor MCP defaults to the already tested `https://api.worldmonitor.app/mcp` endpoint and authenticates with OAuth 2.1 bearer tokens from the World Monitor Pro sign-in flow.
- No API key is requested. Rotating OAuth credentials live in one permission-restricted local file so both the desktop brief collector and the Maritime N2 MCP tool process can refresh them safely without copying credentials into React, prompts, logs, or Buzz.
- World Monitor daily application budgets are independent: 25 attempted `tools/call` requests for all brief updates combined and 25 for direct N2 questions, per local calendar day.
- The 25-call briefing allowance is a ceiling, not a target.
- Cache World Monitor results for 15 minutes by tool name plus RFC 8785-style canonical arguments; a cache hit spends no call.
- There is no autonomous polling. Calls occur only for a brief, an N2 conversation, or an explicit connection test.
- World Monitor, RAG, and Memory failures remain fail-soft and must not prevent an otherwise useful adviser response or partial brief.
- Cloud first and Local first receive the same frozen source ledger after collection.
- No new daemon, database, replication system, OSINT gateway, permanent CTG/JPG hierarchy, or separate planning application.
- Use normal Buzz channels, memberships, and mentions for a mission-specific virtual JPG.
- Do not add generic web search, X/Twitter, Reddit, STRATFOR authentication, or autonomous alerting in this phase.
- Do not add `unsafe`, production `unwrap()`, or production `expect()`.
- Readable frontend text must use the existing rem-based Tailwind tokens.

---

## File and Responsibility Map

### New shared source library

- Create `crates/buzz-command-sources/Cargo.toml` — focused library manifest.
- Create `crates/buzz-command-sources/src/lib.rs` — public exports and shared constants.
- Create `crates/buzz-command-sources/src/mcp_http.rs` — bounded MCP JSON/SSE client with optional API-key header.
- Create `crates/buzz-command-sources/src/world_monitor.rs` — seven-tool allowlist, argument validation, response normalisation, and freshness.
- Create `crates/buzz-command-sources/src/usage.rs` — cross-process 25/25 quota, 15-minute cache, locked atomic JSON state.

### Native advisers and runtime

- Modify `desktop/src-tauri/src/managed_agents/personas.rs` — Logistics/N2 definitions, avatars, doctrine/JPG prompts.
- Modify `desktop/src-tauri/src/managed_agents/personas/tests.rs` — built-in merge, prompt, and identity tests.
- Modify `desktop/src-tauri/src/managed_agents/runtime.rs` — inject protected Command Adviser source settings into command-team processes.
- Modify `desktop/src-tauri/src/managed_agents/env_vars.rs` and `desktop/src-tauri/src/managed_agents/env_vars/tests.rs` — reserve the injected source keys.
- Modify `crates/buzz-dev-mcp/src/lib.rs` — doctrine/knowledge tools for all command advisers and seven World Monitor tools for N2.
- Create `crates/buzz-dev-mcp/src/command_adviser.rs` — sidecar configuration and shared tool-call adapters.
- Modify `crates/buzz-dev-mcp/src/shell.rs` — remove the World Monitor credential from shell child environments.

### Configuration and UI

- Modify `desktop/src-tauri/src/command_services/trusted_lan.rs` and `trusted_lan_tests.rs` — optional World Monitor configuration with safe defaults.
- Create `desktop/src-tauri/src/commands/world_monitor.rs` — Keychain Save/Remove/Test/Status Tauri commands.
- Modify `desktop/src-tauri/src/commands/mod.rs` and `desktop/src-tauri/src/lib.rs` — register commands.
- Modify `desktop/src/shared/api/tauriCommandBrief.ts` — strict World Monitor IPC parsers and calls.
- Create `desktop/src/features/command-console/hooks/useWorldMonitorConnection.ts` — connection state and actions.
- Create `desktop/src/features/command-console/ui/WorldMonitorConnectionCard.tsx` — masked credential setup, test, remove, and daily-use display.
- Modify `desktop/src/features/command-console/ui/CommandConsoleScreen.tsx` — render the connection card.

### Brief contracts, collection, and presentation

- Modify `desktop/src-tauri/src/command_brief/types.rs`, `types_tests.rs` — new advisers, sections, source kind, and seven-specialist contract.
- Modify `desktop/src-tauri/src/command_brief/personas.rs`, `personas_tests.rs` — N2/Logistics brief prompts and permissions.
- Modify `desktop/src-tauri/src/command_brief/sources/retrieval_intents.rs` — doctrine-first then general RAG intents.
- Create `desktop/src-tauri/src/command_brief/sources/world_monitor.rs` — deterministic bounded N2 update plan and candidate conversion.
- Modify `desktop/src-tauri/src/command_brief/sources.rs`, `sources_tests.rs`, and `sources_tests/policy_and_production.rs` — collect World Monitor evidence fail-soft.
- Modify `desktop/src-tauri/src/command_brief/sources/canonical.rs` — admit `world_monitor` provenance and resize source-kind counters.
- Modify `desktop/src-tauri/src/command_brief/sources/command_team_discussions.rs` — accept N2/Logistics outcomes and sections.
- Modify `desktop/src-tauri/src/command_brief/orchestrator.rs`, `orchestrator_tests.rs`, and `orchestrator/assembly.rs` — seven-specialist lifecycle and consolidation.
- Modify `desktop/src/features/command-console/domain/briefContracts.ts` — matching TypeScript contract.
- Modify `desktop/src/features/command-console/domain/commandTeam.ts` — eight-person roster.
- Modify `desktop/src/features/command-console/ui/AdviserInsignia.tsx` — N2 and Logistics symbols.
- Modify `desktop/src/features/command-console/ui/briefPresentation.ts` — labels and decision-first order.
- Modify `desktop/src/features/command-console/ui/DailyCommandBrief.tsx` and its tests — render Intelligence and Logistics.

### End-to-end acceptance

- Modify `desktop/tests/helpers/bridge.ts` — World Monitor IPC state and mock handlers.
- Modify `desktop/tests/e2e/command-team-conversations.spec.ts` — eight reusable native advisers.
- Modify `desktop/tests/e2e/daily-command-brief.spec.ts` — Intelligence/Logistics ordering and fail-soft state.
- Create `desktop/tests/e2e/world-monitor-connection.spec.ts` — credential/status/usage UI.
- Modify `desktop/playwright.config.ts` — include the new smoke spec.
- Modify `docs/command-console/phase-4-daily-command-brief.md` — current seven-specialist and World Monitor behaviour.

---

### Task 1: Expand the Closed Command Team and Brief Contracts

**Files:**
- Modify: `desktop/src-tauri/src/command_brief/types.rs`
- Modify: `desktop/src-tauri/src/command_brief/types_tests.rs`
- Modify: `desktop/src/features/command-console/domain/briefContracts.ts`
- Modify: `desktop/src/features/command-console/domain/briefContracts.test.mjs`
- Modify: `desktop/src/features/command-console/domain/commandTeam.ts`
- Modify: `desktop/src/features/command-console/ui/AdviserInsignia.tsx`
- Modify: `desktop/src/features/command-console/ui/AdviserInsignia.test.mjs`
- Modify: `desktop/src/features/agents/ui/unifiedAgentGroups.test.mjs`

**Interfaces:**
- Produces Rust `AdviserId::{Intelligence, Logistics}`.
- Produces Rust `BriefSection::{Intelligence, Logistics}`.
- Produces Rust `SourceKind::WorldMonitor`.
- Produces matching TypeScript `"intelligence"`, `"logistics"`, and `"world_monitor"` literals.
- Sets `SPECIALIST_COUNT` to `7` and specialist order to Operations, Intelligence, Logistics, Navigation, Daily Routine, Reporting, Plans.

- [x] **Step 1: Write failing Rust closed-contract tests**

Update the fixture sections and contributions and add these assertions:

```rust
assert_eq!(SPECIALIST_COUNT, 7);
assert_eq!(
    SPECIALIST_ADVISERS,
    [
        AdviserId::Operations,
        AdviserId::Intelligence,
        AdviserId::Logistics,
        AdviserId::Navigation,
        AdviserId::DailyRoutine,
        AdviserId::Reporting,
        AdviserId::Plans,
    ]
);
assert_eq!(
    serde_json::to_value(SourceKind::WorldMonitor).expect("serialize source kind"),
    "world_monitor"
);
assert!(brief_value()["sections"].get("intelligence").is_some());
assert!(brief_value()["sections"].get("logistics").is_some());
```

The valid `brief_value()` fixture must contain seven contributions and eleven
section keys. Keep the existing rejection test and add `"unknown_osint"` as an
invalid source kind.

- [x] **Step 2: Run the Rust tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::types_tests
```

Expected: compilation fails because `Intelligence`, `Logistics`, and
`WorldMonitor` do not exist.

- [x] **Step 3: Write failing TypeScript roster and parser tests**

Require the exact adviser order:

```javascript
assert.deepEqual(
  COMMAND_TEAM_PERSONAS.map(({ adviser, personaId }) => [adviser, personaId]),
  [
    ["chief_of_staff", "builtin:command-chief-of-staff"],
    ["operations", "builtin:command-operations"],
    ["intelligence", "builtin:command-intelligence"],
    ["logistics", "builtin:command-logistics"],
    ["navigation", "builtin:command-navigation"],
    ["daily_routine", "builtin:command-daily-routine"],
    ["reporting", "builtin:command-reporting"],
    ["plans", "builtin:command-plans"],
  ],
);
```

Require `ADVISER_IDENTITIES.intelligence.symbol === "intelligence-scan"` and
`ADVISER_IDENTITIES.logistics.symbol === "replenishment"`. Update the brief
parser fixture to seven contributions, eleven sections, and a valid
`world_monitor` source.

- [x] **Step 4: Run the frontend tests and verify RED**

Run:

```bash
cd desktop
pnpm test -- --test-name-pattern="Command Team|brief contract|adviser insignia"
```

Expected: failures report the missing adviser, section, and identity literals.

- [x] **Step 5: Implement the minimum closed enums and UI roster**

Add the Rust variants and exact arrays:

```rust
pub enum AdviserId {
    ChiefOfStaff,
    Operations,
    Intelligence,
    Logistics,
    Navigation,
    DailyRoutine,
    Reporting,
    Plans,
}

pub enum BriefSection {
    Today,
    Operations,
    Intelligence,
    Logistics,
    Navigation,
    DailyRoutine,
    Reports,
    #[serde(rename = "planning_30_60_90")]
    Planning306090,
    Decisions,
    ConflictsAndGaps,
    Sources,
}

pub enum SourceKind {
    Rag,
    Memory,
    WorldMonitor,
    Calendar,
    Reminders,
    Notes,
    File,
}
```

Update every exhaustive match and strict key set in Rust and TypeScript.
Use the existing `Radar` for Operations, `ScanSearch` for N2, and `Fuel` for
Logistics. Preserve the existing sextant asset for Navigation.

- [x] **Step 6: Run focused tests and verify GREEN**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::types_tests
cd desktop
pnpm test -- --test-name-pattern="Command Team|brief contract|adviser insignia"
pnpm typecheck
```

Expected: all commands pass.

- [x] **Step 7: Commit**

```bash
git add desktop/src-tauri/src/command_brief/types.rs \
  desktop/src-tauri/src/command_brief/types_tests.rs \
  desktop/src/features/command-console/domain/briefContracts.ts \
  desktop/src/features/command-console/domain/briefContracts.test.mjs \
  desktop/src/features/command-console/domain/commandTeam.ts \
  desktop/src/features/command-console/ui/AdviserInsignia.tsx \
  desktop/src/features/command-console/ui/AdviserInsignia.test.mjs \
  desktop/src/features/agents/ui/unifiedAgentGroups.test.mjs
git commit -m "feat: expand the command team roster"
```

---

### Task 2: Add Native Logistics and N2 Personas with Doctrine and JPG Behaviour

**Files:**
- Modify: `desktop/src-tauri/src/managed_agents/personas.rs`
- Modify: `desktop/src-tauri/src/managed_agents/personas/tests.rs`
- Modify: `desktop/src-tauri/src/command_brief/sources/command_team_discussions.rs`
- Modify: `desktop/src-tauri/src/command_brief/sources_tests/command_team.rs`

**Interfaces:**
- Produces `builtin:command-intelligence` and `builtin:command-logistics`.
- Extends `command-discussion-outcome-v1` with advisers and sections already added in Task 1.
- Requires the N2 prompt to record ISO 3166-1 alpha-2 codes in outcomes when a country is material; this provides deterministic focus hints to Task 7.

- [x] **Step 1: Write failing built-in persona tests**

Assert the merged built-in list contains both stable IDs once, remains
idempotent on a second merge, and that both definitions have `model: None` and
`runtime: None` so existing configured defaults are inherited.

Assert every command persona prompt contains:

```text
Seek applicable doctrine with search_command_doctrine before substantive advice.
If no applicable doctrine is retrieved, continue with a reasoned assessment.
```

Assert the N2 prompt contains `world_monitor_`, `reported information`,
`observed indicators`, `assumptions`, `assessment`, and
`ISO 3166-1 alpha-2`. Assert the Chief and Operations prompts contain
`mission-specific Buzz channel`, `virtual Joint Planning Group`, and
`@mention`.

- [x] **Step 2: Run persona tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml managed_agents::personas::tests
```

Expected: failures identify the two missing definitions and prompt clauses.

- [x] **Step 3: Write failing discussion-outcome tests**

Add controlled records for:

```json
{
  "adviser": "intelligence",
  "brief_sections": ["intelligence"]
}
```

and:

```json
{
  "adviser": "logistics",
  "brief_sections": ["logistics"]
}
```

Require both persona/adviser mappings, valid slugs, selection, supersession,
and per-adviser limits. Retain rejection of a Logistics body written by the N2
persona.

- [x] **Step 4: Run discussion tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_team_discussions
```

Expected: the new persona IDs are rejected as unknown.

- [x] **Step 5: Implement the two persona definitions and prompt behaviour**

Add symbolic navy/gold SVG data URLs and definitions:

```rust
BuiltInPersona {
    id: "builtin:command-intelligence",
    display_name: "Maritime N2 Adviser",
    avatar_url: Some(COMMAND_INTELLIGENCE_AVATAR),
    system_prompt: COMMAND_INTELLIGENCE_PROMPT,
    name_pool: &["Maritime N2 Adviser"],
    model: None,
    runtime: None,
    default_active: true,
}

BuiltInPersona {
    id: "builtin:command-logistics",
    display_name: "Logistics Adviser",
    avatar_url: Some(COMMAND_LOGISTICS_AVATAR),
    system_prompt: COMMAND_LOGISTICS_PROMPT,
    name_pool: &["Logistics Adviser"],
    model: None,
    runtime: None,
    default_active: true,
}
```

Put N2 after Operations and Logistics after N2 in the built-in order. Extend
`adviser_for_persona` and `adviser_label`. Keep outcome schema v1 and its
current bounds.

- [x] **Step 6: Run focused tests and verify GREEN**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml managed_agents::personas::tests
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_team_discussions
```

Expected: both suites pass.

- [x] **Step 7: Commit**

```bash
git add desktop/src-tauri/src/managed_agents/personas.rs \
  desktop/src-tauri/src/managed_agents/personas/tests.rs \
  desktop/src-tauri/src/command_brief/sources/command_team_discussions.rs \
  desktop/src-tauri/src/command_brief/sources_tests/command_team.rs
git commit -m "feat: add logistics and maritime n2 personas"
```

---

### Task 3: Build the Shared Bounded MCP and World Monitor Client

**Files:**
- Create: `crates/buzz-command-sources/Cargo.toml`
- Create: `crates/buzz-command-sources/src/lib.rs`
- Create: `crates/buzz-command-sources/src/mcp_http.rs`
- Create: `crates/buzz-command-sources/src/world_monitor.rs`
- Modify: `Cargo.toml`
- Modify: `crates/buzz-dev-mcp/Cargo.toml`
- Modify: `desktop/src-tauri/Cargo.toml`

**Interfaces:**
- Produces:

```rust
pub const DEFAULT_WORLD_MONITOR_ENDPOINT: &str = "https://api.worldmonitor.app/mcp";
pub const WORLD_MONITOR_KEYCHAIN_KEY: &str = "command.world-monitor.api-key";

pub enum WorldMonitorTool {
    CountryRisk,
    ConflictEvents,
    MilitaryPosture,
    NewsIntelligence,
    MaritimeActivity,
    ChokepointStatus,
    SupplyChainData,
}

pub struct WorldMonitorRequest {
    pub tool: WorldMonitorTool,
    pub arguments: serde_json::Value,
}

pub struct NormalizedWorldMonitorEvidence {
    pub tool: WorldMonitorTool,
    pub arguments: serde_json::Value,
    pub payload: serde_json::Value,
    pub retrieved_at: chrono::DateTime<chrono::Utc>,
    pub source_time: Option<chrono::DateTime<chrono::Utc>>,
    pub freshness: WorldMonitorFreshness,
}
```

- Produces async `McpHttpClient::list_tools` and
  `McpHttpClient::call_tool`.
- Accepts JSON and SSE MCP responses; rejects redirects, non-HTTPS World
  Monitor endpoints, responses over 2 MiB, invalid JSON-RPC IDs, and MCP
  `isError`.

- [x] **Step 1: Create the crate manifest and failing transport tests**

Register `crates/buzz-command-sources` as a workspace member and dependency.
Use existing workspace `tokio`, `reqwest`, `serde`, `serde_json`, `chrono`,
`sha2`, and `url`; add `thiserror`, `zeroize`, `serde_jcs = "0.2"`,
`atomic-write-file = "0.3"`, `fs2 = "0.4.3"`, and `tempfile` for
errors/state/tests.

Write fake HTTP-server tests named:

- `list_tools_sends_world_monitor_header_without_logging_it`
- `call_tool_accepts_json_and_sse_results`
- `call_tool_rejects_redirects_and_oversized_bodies`
- `call_tool_maps_401_429_timeout_and_mcp_error`
- `endpoint_accepts_only_https_world_monitor_mcp`

The fake server must inspect the real request header and return controlled
wire responses; do not assert only on a mock invocation count.

- [x] **Step 2: Run transport tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-command-sources mcp_http
```

Expected: compilation fails because the client types do not exist.

- [x] **Step 3: Write failing tool and freshness tests**

Require the exact seven upstream tool names:

```rust
assert_eq!(WorldMonitorTool::CountryRisk.as_str(), "get_country_risk");
assert_eq!(WorldMonitorTool::ConflictEvents.as_str(), "get_conflict_events");
assert_eq!(WorldMonitorTool::MilitaryPosture.as_str(), "get_military_posture");
assert_eq!(WorldMonitorTool::NewsIntelligence.as_str(), "get_news_intelligence");
assert_eq!(WorldMonitorTool::MaritimeActivity.as_str(), "get_maritime_activity");
assert_eq!(WorldMonitorTool::ChokepointStatus.as_str(), "get_chokepoint_status");
assert_eq!(WorldMonitorTool::SupplyChainData.as_str(), "get_supply_chain_data");
```

Require `country_code` to be two uppercase ASCII letters, `limit` to be
`1..=30`, news topic to be one of `conflict`, `economy`, `cyber`, `nuclear`,
`intelligence`, `maritime`, and reject `jmespath` from application-generated
arguments. Test RFC3339, Unix-second, Unix-millisecond, zero, future, absent,
24-hour tactical, and seven-day strategic freshness cases.

- [x] **Step 4: Run tool tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-command-sources world_monitor
```

Expected: compilation fails because the tool and freshness types do not exist.

- [x] **Step 5: Implement the minimum client**

Use a Reqwest client with redirects disabled and a ten-second timeout. Send:

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/call",
  "params": {
    "name": "get_country_risk",
    "arguments": {
      "country_code": "PH"
    }
  }
}
```

Set `Accept: application/json, text/event-stream`,
`Content-Type: application/json`, and `X-WorldMonitor-Key`. Parse the final SSE
`data:` JSON object when content type is event stream. Keep the API key in a
zeroizing local wrapper and make every `Debug` implementation omit it.

Use 24 hours for Conflict Events, Military Posture, News Intelligence, and
Maritime Activity. Use seven days for Country Risk, Chokepoint Status, and
Supply Chain Data. A missing, zero, more-than-five-minutes-future, or unparsable
source time is `Unknown`; an older valid source time is `Stale`.

- [x] **Step 6: Run crate tests and verify GREEN**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-command-sources
cargo clippy -p buzz-command-sources --all-targets -- -D warnings
```

Expected: both commands pass with no secret in failure output.

- [x] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock \
  crates/buzz-command-sources \
  crates/buzz-dev-mcp/Cargo.toml \
  desktop/src-tauri/Cargo.toml
git commit -m "feat: add bounded command source client"
```

---

### Task 4: Implement the Shared 25/25 Usage Ledger and Cache

**Files:**
- Create: `crates/buzz-command-sources/src/usage.rs`
- Modify: `crates/buzz-command-sources/src/lib.rs`
- Modify: `crates/buzz-command-sources/Cargo.toml`

**Interfaces:**
- Produces:

```rust
pub enum UsagePool {
    Brief,
    Direct,
}

pub struct UsageSnapshot {
    pub local_date: String,
    pub brief_used: u8,
    pub direct_used: u8,
}

pub enum UsageAdmission {
    Cached(NormalizedWorldMonitorEvidence),
    Reserved { cache_key: String, snapshot: UsageSnapshot },
}

pub struct WorldMonitorUsageLedger {
    state_path: std::path::PathBuf,
}
```

- `admit(pool, request, now_local)` performs cache lookup and quota reservation
  under one exclusive file lock.
- `store_success(cache_key, evidence, now_local)` atomically stores a bounded
  cache result.

- [x] **Step 1: Write failing ledger tests**

Create tests named:

- `brief_and_direct_have_independent_25_call_limits`
- `all_brief_runs_share_one_local_day_pool`
- `cache_hit_within_15_minutes_spends_no_call`
- `cache_miss_after_15_minutes_reserves_next_call`
- `next_local_day_resets_counts_and_cache`
- `failed_outbound_attempt_remains_counted`
- `concurrent_ledgers_never_admit_call_26`
- `state_file_is_owner_only_and_never_contains_api_key`
- `corrupt_or_oversized_state_fails_closed_without_panicking`

Use two `WorldMonitorUsageLedger` instances pointing to the same temporary
path in the concurrency test.

- [x] **Step 2: Run ledger tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-command-sources usage
```

Expected: compilation fails because the usage types do not exist.

- [x] **Step 3: Implement locked atomic state**

Persist this bounded shape:

```json
{
  "version": 1,
  "localDate": "2026-07-28",
  "briefUsed": 0,
  "directUsed": 0,
  "cache": {}
}
```

Hash `tool-name + ":" + serde_jcs(canonical-arguments)` for cache keys. Limit
the cache to 64 entries and the state file to 2 MiB. Acquire an exclusive lock
on a sibling `.lock` file, re-read state inside the lock, reserve before the
network request, and persist via a same-directory temporary file plus rename.
On Unix, set file mode `0o600`. Do not decrement a reservation when the
network request fails.

- [x] **Step 4: Run tests and verify GREEN**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-command-sources usage
cargo test -p buzz-command-sources
```

Expected: all tests pass.

- [x] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/buzz-command-sources
git commit -m "feat: enforce world monitor daily budgets"
```

---

### Task 5: Add Keychain Configuration, Connection Diagnostics, and UI

**Files:**
- Modify: `desktop/src-tauri/src/command_services/trusted_lan.rs`
- Modify: `desktop/src-tauri/src/command_services/trusted_lan_tests.rs`
- Create: `desktop/src-tauri/src/commands/world_monitor.rs`
- Modify: `desktop/src-tauri/src/commands/mod.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src/shared/api/tauriCommandBrief.ts`
- Modify: `desktop/src/shared/api/tauriCommandBrief.test.mjs`
- Create: `desktop/src/features/command-console/hooks/useWorldMonitorConnection.ts`
- Create: `desktop/src/features/command-console/hooks/useWorldMonitorConnection.hook.test.mjs`
- Create: `desktop/src/features/command-console/ui/WorldMonitorConnectionCard.tsx`
- Create: `desktop/src/features/command-console/ui/WorldMonitorConnectionCard.test.mjs`
- Modify: `desktop/src/features/command-console/ui/CommandConsoleScreen.tsx`

**Interfaces:**
- Produces Tauri commands:

```rust
get_world_monitor_connection() -> WorldMonitorConnectionView
save_world_monitor_api_key(api_key: String) -> WorldMonitorConnectionView
remove_world_monitor_api_key() -> WorldMonitorConnectionView
test_world_monitor_connection() -> WorldMonitorConnectionView
```

- `WorldMonitorConnectionView` exposes only:

```rust
pub struct WorldMonitorConnectionView {
    pub endpoint: String,
    pub status: WorldMonitorConnectionStatus,
    pub brief_used: u8,
    pub brief_limit: u8,
    pub direct_used: u8,
    pub direct_limit: u8,
}
```

- [x] **Step 1: Write failing native configuration and command tests**

Require legacy `trusted-lan-sources.json` files without `world_monitor` to load
with the default endpoint and keychain identifier. Require config save to
preserve Memory, RAG, LiteLLM, OpenAI, and routing fields.

Use an injected fake secret store and fake `McpHttpClient` to prove:

- save accepts `wm_live_` plus at least 16 total characters;
- save rejects whitespace, control characters, more than 512 bytes, and other
  prefixes;
- get returns `not_configured` or `configured` without the key;
- remove is idempotent;
- test uses `tools/list`, not `tools/call`;
- `401`, `429`, timeout, malformed, and success map to stable redacted status;
- returned serialised JSON never contains `wm_live_`.

- [x] **Step 2: Run native tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml world_monitor
cargo test --manifest-path desktop/src-tauri/Cargo.toml trusted_lan
```

Expected: missing module, commands, and config fields.

- [x] **Step 3: Write failing API, hook, and card tests**

Require exact IPC response keys:

```typescript
type WorldMonitorConnection = {
  readonly endpoint: string;
  readonly status:
    | "not_configured"
    | "configured"
    | "connected"
    | "unavailable"
    | "unauthorised"
    | "quota_limited";
  readonly briefUsed: number;
  readonly briefLimit: 25;
  readonly directUsed: number;
  readonly directLimit: 25;
};
```

The card test must assert:

- password input never receives a value from native status;
- Save sends the typed key once and clears component state;
- Test connection is disabled until configured;
- Remove returns to not configured;
- text shows `Brief 3/25` and `Direct questions 4/25`;
- no arbitrary pixel text classes are introduced.

- [x] **Step 4: Run frontend tests and verify RED**

Run:

```bash
cd desktop
pnpm test -- --test-name-pattern="World Monitor"
```

Expected: imports fail because the API, hook, and card do not exist.

- [x] **Step 5: Implement native and frontend configuration**

Use:

```rust
let mut api_key = api_key;
SecretStore::shared(crate::app_state::keyring_service())
    .store(WORLD_MONITOR_KEYCHAIN_KEY, &api_key)?;
zeroize::Zeroize::zeroize(&mut api_key);
```

Load only with `SecretStore::load` inside native code. Zeroize the request
string after save/test. `test_world_monitor_connection` calls `tools/list` with
the stored key and does not access either 25-call pool.

Render the connection card below the advisory notice and above the Daily
Command Brief. Keep it compact and visually consistent with the navy/gold
Command Adviser theme.

- [x] **Step 6: Run focused tests and verify GREEN**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml world_monitor
cargo test --manifest-path desktop/src-tauri/Cargo.toml trusted_lan
cd desktop
pnpm test -- --test-name-pattern="World Monitor"
pnpm typecheck
pnpm check:px-text
```

Expected: all commands pass.

- [x] **Step 7: Commit**

```bash
git add desktop/src-tauri/src/command_services/trusted_lan.rs \
  desktop/src-tauri/src/command_services/trusted_lan_tests.rs \
  desktop/src-tauri/src/commands/world_monitor.rs \
  desktop/src-tauri/src/commands/mod.rs \
  desktop/src-tauri/src/lib.rs \
  desktop/src/shared/api/tauriCommandBrief.ts \
  desktop/src/shared/api/tauriCommandBrief.test.mjs \
  desktop/src/features/command-console/hooks/useWorldMonitorConnection.ts \
  desktop/src/features/command-console/hooks/useWorldMonitorConnection.hook.test.mjs \
  desktop/src/features/command-console/ui/WorldMonitorConnectionCard.tsx \
  desktop/src/features/command-console/ui/WorldMonitorConnectionCard.test.mjs \
  desktop/src/features/command-console/ui/CommandConsoleScreen.tsx
git commit -m "feat: configure world monitor in command adviser"
```

---

### Task 6: Expose Doctrine and World Monitor Tools to Managed Conversations

**Files:**
- Create: `crates/buzz-dev-mcp/src/command_adviser.rs`
- Modify: `crates/buzz-dev-mcp/src/lib.rs`
- Modify: `crates/buzz-dev-mcp/src/shell.rs`
- Modify: `desktop/src-tauri/src/managed_agents/runtime.rs`
- Modify: `desktop/src-tauri/src/managed_agents/runtime/tests.rs`
- Modify: `desktop/src-tauri/src/managed_agents/env_vars.rs`
- Modify: `desktop/src-tauri/src/managed_agents/env_vars/tests.rs`

**Interfaces:**
- Environment set by desktop:

```text
COMMAND_ADVISER_PERSONA_ID
COMMAND_ADVISER_RAG_URL
COMMAND_ADVISER_WORLD_MONITOR_ENDPOINT
COMMAND_ADVISER_WORLD_MONITOR_USAGE_PATH
COMMAND_ADVISER_WORLD_MONITOR_API_KEY
```

- MCP tools available to all command advisers:
  `search_command_doctrine(query, top_k)` and
  `search_command_knowledge(query, collections, top_k)`.
- MCP tools permitted only when persona ID is `builtin:command-intelligence`:
  `world_monitor_country_risk`,
  `world_monitor_conflict_events`,
  `world_monitor_military_posture`,
  `world_monitor_news_intelligence`,
  `world_monitor_maritime_activity`,
  `world_monitor_chokepoint_status`,
  `world_monitor_supply_chain_data`.

- [x] **Step 1: Write failing sidecar tool tests**

Construct `CommandAdviserTools` with a fake RAG MCP server and fake World
Monitor server. Prove:

- doctrine always sends `collections: ["ADF Doctrine"]`;
- broader knowledge preserves a bounded explicit collection list;
- N2 calls use `UsagePool::Direct`;
- a non-N2 persona receives an MCP error before an outbound request;
- missing RAG returns a concise tool error and does not terminate the server;
- World Monitor cache hits return the original retrieval time;
- the seven methods emit only their approved argument fields.

- [x] **Step 2: Run sidecar tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-dev-mcp command_adviser
```

Expected: missing module and tools.

- [x] **Step 3: Write failing runtime environment tests**

Require:

- all eight command personas receive persona ID, RAG URL, endpoint, and usage
  path;
- only N2 receives the API key;
- a missing key omits the secret variable but still starts N2;
- all five variable names are reserved from user/persona overrides;
- `buzz-dev-mcp` shell child removes
  `COMMAND_ADVISER_WORLD_MONITOR_API_KEY`.

- [x] **Step 4: Run runtime tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml managed_agents::runtime::tests
cargo test --manifest-path desktop/src-tauri/Cargo.toml managed_agents::env_vars::tests
cargo test -p buzz-dev-mcp shell
```

Expected: assertions fail because protected variables are not installed or
removed.

- [x] **Step 5: Implement sidecar tools and protected injection**

In managed-agent spawn, call one helper after user environment layering:

```rust
apply_command_adviser_source_env(
    &mut command,
    app,
    record.persona_id.as_deref(),
)?;
```

The helper removes every Command Adviser variable first, then sets the
non-secret values for recognised command personas. It loads the Keychain key
only for `builtin:command-intelligence`. Do not log the key or include it in
the spawn hash.

Each World Monitor RMCP method builds a typed `WorldMonitorRequest`, admits it
through the direct pool, executes only when reserved, stores successful
evidence, and returns compact JSON text. Each method returns cached evidence
without an outbound call.

- [x] **Step 6: Run focused tests and verify GREEN**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-dev-mcp command_adviser
cargo test -p buzz-dev-mcp shell
cargo test --manifest-path desktop/src-tauri/Cargo.toml managed_agents::runtime::tests
cargo test --manifest-path desktop/src-tauri/Cargo.toml managed_agents::env_vars::tests
```

Expected: all commands pass.

- [x] **Step 7: Commit**

```bash
git add crates/buzz-dev-mcp/src/command_adviser.rs \
  crates/buzz-dev-mcp/src/lib.rs \
  crates/buzz-dev-mcp/src/shell.rs \
  desktop/src-tauri/src/managed_agents/runtime.rs \
  desktop/src-tauri/src/managed_agents/runtime/tests.rs \
  desktop/src-tauri/src/managed_agents/env_vars.rs \
  desktop/src-tauri/src/managed_agents/env_vars/tests.rs
git commit -m "feat: add command adviser evidence tools"
```

---

### Task 7: Collect Doctrine-First RAG and Comprehensive N2 Brief Evidence

**Files:**
- Modify: `desktop/src-tauri/src/command_brief/sources/retrieval_intents.rs`
- Create: `desktop/src-tauri/src/command_brief/sources/world_monitor.rs`
- Modify: `desktop/src-tauri/src/command_brief/sources.rs`
- Modify: `desktop/src-tauri/src/command_brief/sources_tests.rs`
- Modify: `desktop/src-tauri/src/command_brief/sources_tests/policy_and_production.rs`
- Modify: `desktop/src-tauri/src/command_brief/sources/canonical.rs`
- Modify: `desktop/src-tauri/src/command_brief/orchestrator/providers.rs`

**Interfaces:**
- `FixedRetrievalIntent` produces separate `doctrine_query()` and
  `context_query()`.
- `WorldMonitorBriefCollector::collect` returns:

```rust
pub struct WorldMonitorBriefBatch {
    pub candidates: Vec<CandidateSource>,
    pub limitations: Vec<String>,
    pub quota_limited: bool,
}
```

- Deterministic daily request plan:
  - eight global summary calls: conflict; conflict news; economy news;
    intelligence news; maritime news; military posture; chokepoints; supply
    chain;
  - for up to five valid focus country codes found in the CO request or active
    command-team outcomes: country risk, maritime activity, and country news;
  - maximum 23 planned calls before cache/quota admission.

- [x] **Step 1: Write failing doctrine-intent tests**

For every specialist, require:

```rust
assert!(intent.doctrine_query().contains("applicable ADF doctrine"));
assert!(intent.context_query().contains("CO request:"));
assert_eq!(intent.doctrine_collections(), &["ADF Doctrine"]);
```

Test a RAG catalogue without `ADF Doctrine`: collection proceeds with the
general query and one bounded limitation. Test doctrine call failure:
subsequent general RAG and Memory calls still occur.

- [x] **Step 2: Run doctrine tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml retrieval_intents
cargo test --manifest-path desktop/src-tauri/Cargo.toml sources_tests
```

Expected: missing doctrine/context fields and one-call behaviour.

- [x] **Step 3: Write failing World Monitor collection tests**

Use a fake World Monitor executor and real usage ledger. Assert:

- no focus code produces the exact eight-call global plan;
- `(PH)` and `"country_code":"JP"` produce country calls for PH and JP;
- duplicated and invalid codes are ignored;
- five codes cap the plan at 23;
- cache hits and remaining daily budget reduce outbound calls;
- no API key returns an Intelligence limitation without calling the network;
- one failed tool preserves other candidates;
- `401`, `429`, timeout, malformed, and stale responses degrade Intelligence
  but do not return `SourceCollectionError`;
- source IDs hash provider, tool, canonical arguments, and retrieval identity;
- zero timestamps are labelled `freshness=unknown`;
- `SourceKind::WorldMonitor` survives canonicalisation and is available to
  Intelligence and Logistics.

- [x] **Step 4: Run World Monitor source tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml world_monitor
cargo test --manifest-path desktop/src-tauri/Cargo.toml canonical
```

Expected: missing collector and source-kind counter handling.

- [x] **Step 5: Implement doctrine-first collection**

For each specialist:

1. call RAG with `collections: ["ADF Doctrine"]` when present;
2. admit valid doctrine candidates;
3. call RAG again with all observed logical collections;
4. call Memory with the context query;
5. continue after a doctrine-specific empty result or error.

Do not set global `rag_available = false` merely because the doctrine call
failed. Set it false only after the broader RAG call fails.

- [x] **Step 6: Implement the N2 update collector**

Load the World Monitor key natively, build the deterministic plan, admit each
request through `UsagePool::Brief`, and append successful evidence as
`CandidateSource`. A request failure adds one deduplicated Intelligence
limitation. `QuotaExceeded` stops remaining requests for that run.

Resize canonical source-kind arrays from six to seven and make retention order:

```rust
Calendar, Reminders, Notes, File, Memory, Rag, WorldMonitor
```

Raise the total canonical ledger limit from 48 to 72 so the normal eight global
and fifteen country-focus results do not displace the existing RAG, Memory, and
Apple evidence. Omission counts for World Monitor degrade
`BriefSection::Intelligence`, not the whole run.

In `orchestrator/providers.rs`, construct the optional
`WorldMonitorBriefCollector` from the app configuration, Keychain secret, and
shared usage path, then attach it to the existing `SourceCollector`. Missing
configuration attaches an unavailable collector rather than rejecting the
source backend.

- [x] **Step 7: Run focused tests and verify GREEN**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml retrieval_intents
cargo test --manifest-path desktop/src-tauri/Cargo.toml sources_tests
cargo test --manifest-path desktop/src-tauri/Cargo.toml canonical
cargo test --manifest-path desktop/src-tauri/Cargo.toml world_monitor
```

Expected: all commands pass.

- [x] **Step 8: Commit**

```bash
git add desktop/src-tauri/src/command_brief/sources/retrieval_intents.rs \
  desktop/src-tauri/src/command_brief/sources/world_monitor.rs \
  desktop/src-tauri/src/command_brief/sources.rs \
  desktop/src-tauri/src/command_brief/sources_tests.rs \
  desktop/src-tauri/src/command_brief/sources_tests/policy_and_production.rs \
  desktop/src-tauri/src/command_brief/sources/canonical.rs \
  desktop/src-tauri/src/command_brief/orchestrator/providers.rs
git commit -m "feat: collect doctrine and n2 brief evidence"
```

---

### Task 8: Run Seven Specialists and Render the Decision-First Brief

**Files:**
- Modify: `desktop/src-tauri/src/command_brief/personas.rs`
- Modify: `desktop/src-tauri/src/command_brief/personas_tests.rs`
- Modify: `desktop/src-tauri/src/command_brief/orchestrator.rs`
- Modify: `desktop/src-tauri/src/command_brief/orchestrator/assembly.rs`
- Modify: `desktop/src-tauri/src/command_brief/orchestrator_tests.rs`
- Modify: `desktop/src-tauri/src/command_brief/lmstudio_tests.rs`
- Modify: `desktop/src-tauri/src/command_brief/cloud_tests.rs`
- Modify: `desktop/src/features/command-console/ui/briefPresentation.ts`
- Modify: `desktop/src/features/command-console/ui/DailyCommandBrief.tsx`
- Modify: `desktop/src/features/command-console/ui/DailyCommandBrief.test.mjs`
- Modify: `desktop/src/features/command-console/ui/BriefEvidenceDisclosure.tsx`
- Modify: `desktop/src/features/command-console/ui/AdviserContributionCard.tsx`

**Interfaces:**
- N2 permits RAG, Memory, and World Monitor sources and the Intelligence
  section.
- Logistics permits RAG, Memory, and World Monitor sources and the Logistics
  section.
- Specialist order exactly matches Task 1.
- Visible section order exactly matches the approved design.

- [x] **Step 1: Write failing persona and orchestration tests**

Require seven definitions in this order:

```rust
vec![
    AdviserId::Operations,
    AdviserId::Intelligence,
    AdviserId::Logistics,
    AdviserId::Navigation,
    AdviserId::DailyRoutine,
    AdviserId::Reporting,
    AdviserId::Plans,
]
```

Require N2 and Logistics prompts to emit their exact adviser/section strings,
cite ledger IDs, preserve dissent, and keep proposals pending. Run the fake
orchestrator and assert seven specialist executions, a Chief input containing
seven validated contributions, and partial completion when N2 fails.

- [x] **Step 2: Run Rust tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::personas_tests
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::orchestrator_tests
```

Expected: five-specialist expectations fail.

- [x] **Step 3: Write failing presentation tests**

Render a complete brief and assert index order:

```javascript
const order = [
  "brief-section-decisions",
  "brief-section-today",
  "brief-section-operations",
  "brief-section-intelligence",
  "brief-section-logistics",
  "brief-section-navigation",
  "brief-section-daily-routine",
  "brief-section-reports",
  "brief-section-planning-30-60-90",
];
```

Require the labels `Intelligence and operating environment` and
`Logistics and sustainment`. Require World Monitor evidence to appear only
inside the collapsed Evidence and system status disclosure.

- [x] **Step 4: Run frontend tests and verify RED**

Run:

```bash
cd desktop
pnpm test -- --test-name-pattern="Daily Command Brief"
```

Expected: missing section cards and five-specialist parser fixture failures.

- [x] **Step 5: Implement specialist definitions and orchestration**

Add:

```rust
const INTELLIGENCE_SOURCES: &[SourceKind] = &[
    SourceKind::Rag,
    SourceKind::Memory,
    SourceKind::WorldMonitor,
];

const LOGISTICS_SOURCES: &[SourceKind] = &[
    SourceKind::Rag,
    SourceKind::Memory,
    SourceKind::WorldMonitor,
];
```

Keep specialist execution sequential by default and preserve the existing
maximum concurrency of two. Update all five-specialist copy, collection sizes,
contribution maps, dissent caps, and mock outputs to seven.

- [x] **Step 6: Implement decision-first presentation**

Insert Intelligence immediately after Operations and Logistics immediately
after Intelligence. Add exact labels in `briefPresentation.ts`. Preserve
Decisions first and keep Sources, individual contributions, lifecycle,
warnings, and system provenance inside `BriefEvidenceDisclosure`.

- [x] **Step 7: Run focused tests and verify GREEN**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::personas_tests
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::orchestrator_tests
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::lmstudio_tests
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::cloud_tests
cd desktop
pnpm test -- --test-name-pattern="Daily Command Brief"
pnpm typecheck
```

Expected: all commands pass.

- [x] **Step 8: Commit**

```bash
git add desktop/src-tauri/src/command_brief/personas.rs \
  desktop/src-tauri/src/command_brief/personas_tests.rs \
  desktop/src-tauri/src/command_brief/orchestrator.rs \
  desktop/src-tauri/src/command_brief/orchestrator/assembly.rs \
  desktop/src-tauri/src/command_brief/orchestrator_tests.rs \
  desktop/src-tauri/src/command_brief/lmstudio_tests.rs \
  desktop/src-tauri/src/command_brief/cloud_tests.rs \
  desktop/src/features/command-console/ui/briefPresentation.ts \
  desktop/src/features/command-console/ui/DailyCommandBrief.tsx \
  desktop/src/features/command-console/ui/DailyCommandBrief.test.mjs \
  desktop/src/features/command-console/ui/BriefEvidenceDisclosure.tsx \
  desktop/src/features/command-console/ui/AdviserContributionCard.tsx
git commit -m "feat: add intelligence and logistics to command briefs"
```

---

### Task 9: Complete E2E, Live Acceptance, Documentation, and Full Verification

**Files:**
- Modify: `desktop/tests/helpers/bridge.ts`
- Modify: `desktop/tests/e2e/command-team-conversations.spec.ts`
- Modify: `desktop/tests/e2e/daily-command-brief.spec.ts`
- Create: `desktop/tests/e2e/world-monitor-connection.spec.ts`
- Modify: `desktop/playwright.config.ts`
- Modify: `docs/command-console/phase-4-daily-command-brief.md`
- Modify: `docs/superpowers/plans/2026-07-28-command-team-logistics-intelligence.md`

**Interfaces:**
- E2E bridge returns strict World Monitor status and records Save/Test/Remove
  calls without storing the test credential in snapshots.
- Live acceptance uses the real signed Command Adviser app, current
  cloud/local route, trusted-LAN RAG/Memory, and World Monitor subscription.

- [x] **Step 1: Write failing E2E specs**

The connection spec must:

1. open Command Adviser;
2. assert not configured;
3. type a sentinel `wm_live_e2e_not_real`;
4. Save and confirm the input clears;
5. Test connection and show connected from the mock;
6. display Brief `0/25` and Direct questions `0/25`;
7. Remove and return to not configured;
8. assert the sentinel is absent from page text and captured mock state.

Update conversation E2E to Message N2 and Logistics twice each and prove reuse.
Update brief E2E to assert the nine visible section cards in exact order and a
World Monitor failure warning inside the collapsed evidence disclosure.

- [x] **Step 2: Run E2E specs and verify RED**

Run:

```bash
. ./bin/activate-hermit
cd desktop
pnpm build:e2e
pnpm exec playwright test --project=smoke \
  command-team-conversations.spec.ts \
  daily-command-brief.spec.ts \
  world-monitor-connection.spec.ts
```

Expected: missing mock commands, two agents, and two brief sections.

- [x] **Step 3: Implement mock bridge support and documentation**

Add exact mock handlers for the four World Monitor Tauri commands and register
the spec in the smoke project. Update the operator document with:

- eight standing advisers;
- seven brief specialists;
- doctrine-first but non-blocking behaviour;
- Keychain setup and connection test;
- 25/25 local pools and 15-minute cache;
- eight global update calls plus up to five country focus expansions;
- fail-soft World Monitor behaviour;
- no background polling.

- [x] **Step 4: Run E2E specs and verify GREEN**

Run the same Playwright command from Step 2.

Expected: all three specs pass.

- [x] **Step 5: Run the full automated gates**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-command-sources
cargo test -p buzz-dev-mcp
cargo test --manifest-path desktop/src-tauri/Cargo.toml
cd desktop
pnpm test
pnpm typecheck
pnpm check
pnpm test:e2e:smoke
cd ..
just ci
git diff --check
git status --short
```

Expected: every test and check passes; `git status --short` lists only the
intended Task 9 files before commit.

- [ ] **Step 6: Perform live Keychain and N2 DM acceptance**

Launch the signed/development Command Adviser build. If the World Monitor key
is not yet in the application Keychain, stop here for the user to paste it into
the World Monitor card; do not request it in chat or capture it in terminal
output.

Then:

1. Test connection and confirm Connected.
2. DM Maritime N2: ask for a Philippines operating-environment update three
   months before deployment.
3. Inspect the signed reply for ADF Doctrine retrieval, World Monitor country
   risk/news/conflict/maritime evidence, reporting/indicator/assumption/
   assessment separation, and ISO code PH.
4. Accept one material outcome and confirm `Recorded for future briefs`.
5. DM Logistics about tanker sustainment implications for that deployment.
6. Accept one outcome and confirm it is recorded.
7. Reopen each adviser from My Agents and confirm the existing DM is reused.

- [ ] **Step 7: Perform live brief and failure acceptance**

1. Generate Cloud first and confirm seven contributions, Intelligence and
   Logistics sections, evidence citations, actual provider/model, and usage
   counts no greater than 25.
2. Switch to Local first and regenerate within 15 minutes; confirm identical
   World Monitor queries use cache and do not increase the count.
3. Temporarily remove the World Monitor key through the UI.
4. Generate another brief and confirm Intelligence is degraded while Apple,
   RAG, Memory, other advisers, Chief consolidation, and signed publication
   remain usable.
5. Restore the key through the UI and re-test connection.

- [ ] **Step 8: Record high-value Memory MCP events**

With agent `CODEX`, record:

- the final shared-client/usage-ledger architecture and Keychain boundary;
- any real World Monitor protocol, freshness, quota, or model gotcha found in
  live acceptance;
- the final verified commit, test gates, and any explicitly deferred defect.

Do not record the API key, raw session material, or noisy test-by-test events.

- [ ] **Step 9: Mark the plan complete and commit**

Check every completed box in this plan, then:

```bash
git add desktop/tests/helpers/bridge.ts \
  desktop/tests/e2e/command-team-conversations.spec.ts \
  desktop/tests/e2e/daily-command-brief.spec.ts \
  desktop/tests/e2e/world-monitor-connection.spec.ts \
  desktop/playwright.config.ts \
  docs/command-console/phase-4-daily-command-brief.md \
  docs/superpowers/plans/2026-07-28-command-team-logistics-intelligence.md
git commit -m "test: verify logistics and intelligence advisers"
```

- [ ] **Step 10: Push and update the draft PR**

```bash
. ./bin/activate-hermit
git push origin codex/command-team-conversations
```

Update the existing draft PR for `codex/command-team-conversations` with the
two new advisers, doctrine behaviour, World Monitor configuration, 25/25
budgets, automated gates, and live acceptance. If no draft PR exists for the
branch, create one targeting the branch's current upstream base.
