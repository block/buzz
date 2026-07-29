# Public Relay Agent Registry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Buzz Desktop v0.5.0 display registered public Relay Agents while keeping their Relay and Channel role as `member`, and provide idempotent local commands that create, configure, authorize, start, resume, and supervise future public Agents only in explicitly named Channels.

**Architecture:** The PoC deployment owns a versioned `platform/agents/public-agents.json` registry and atomically projects a sanitized copy into the Buzz app-data directory. A new Tauri read-only command loads that projection; the React Agents view merges registered `member` identities with kind `10100` Relay Agents and live Channel membership. Node-based deployment utilities own validation and state transitions, while the existing shell supervisor discovers Runner IDs from the registry instead of a fixed five-role list.

**Tech Stack:** Rust/Tauri 2, TypeScript/React 19, TanStack Query, Node.js ESM and `node:test`, Bash, jq, tmux, Buzz CLI, buzz-admin, buzz-acp.

## Global Constraints

- Stay on Buzz v0.5.0; do not roll back.
- Keep current Product, Tech, Coding, CR, and QA public/private identities and work directories unchanged.
- Keep Relay and Channel role `member`; do not publish synthetic kind `10100` events.
- `create-public-agent` requires one or more explicit `--channel` arguments and joins no other Channel.
- Default runtime is local Claude Code through the existing custom Provider, `respond_to=anyone`, owner-only DM boundary, and an isolated work directory.
- Registry files must contain no private key, `nsec`, Provider API Key, token, or copied environment secret.
- Identity files remain separate and mode `0600`.
- The deployment source registry is `platform/agents/public-agents.json`.
- The Desktop projection is `~/Library/Application Support/xyz.block.buzz.app/agents/public-relay-agents.json`.
- The PoC `platform/` directory is outside the `vendor/buzz` Git repository; Git commits cover Desktop source and documentation, while platform changes are verified in place.

---

### Task 1: Tauri Public Registry Reader

**Files:**
- Create: `desktop/src-tauri/src/public_relay_agents.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src/shared/api/types.ts`
- Modify: `desktop/src/shared/api/tauri.ts`
- Modify: `desktop/src/features/agents/hooks.ts`

**Interfaces:**
- Produces Rust command: `list_public_relay_agents(app: tauri::AppHandle) -> Result<Vec<PublicRelayAgentRegistration>, String>`.
- Produces TypeScript type: `PublicRelayAgentRegistration`.
- Produces frontend API: `listPublicRelayAgents(): Promise<PublicRelayAgentRegistration[]>`.
- Produces query hook: `usePublicRelayAgentsQuery()`.

- [ ] **Step 1: Write Rust tests for missing, valid, and invalid registry files**

Add tests around a pure path-based loader:

```rust
#[test]
fn missing_registry_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load_public_relay_agents_from_path(&dir.path().join("missing.json")).unwrap(), vec![]);
}

#[test]
fn valid_registry_normalizes_pubkeys_and_rejects_no_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("public-relay-agents.json");
    std::fs::write(
        &path,
        r#"{"version":1,"agents":[{"id":"product","name":"Product Agent","pubkey":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","channelIds":["channel-a"],"state":"active","enabled":true}]}"#,
    ).unwrap();
    let agents = load_public_relay_agents_from_path(&path).unwrap();
    assert_eq!(agents[0].pubkey, "a".repeat(64));
}

#[test]
fn invalid_version_and_duplicate_identity_fail_loudly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("public-relay-agents.json");
    std::fs::write(&path, r#"{"version":2,"agents":[]}"#).unwrap();
    let version_error = load_public_relay_agents_from_path(&path).unwrap_err();
    assert!(version_error.contains("unsupported public relay agent registry version 2"));

    std::fs::write(
        &path,
        format!(
            r#"{{"version":1,"agents":[
              {{"id":"one","name":"One","pubkey":"{}","channelIds":["channel-a"],"state":"active","enabled":true}},
              {{"id":"one","name":"Two","pubkey":"{}","channelIds":["channel-b"],"state":"active","enabled":true}}
            ]}}"#,
            "a".repeat(64),
            "b".repeat(64),
        ),
    ).unwrap();
    let duplicate_error = load_public_relay_agents_from_path(&path).unwrap_err();
    assert!(duplicate_error.contains("duplicate public relay agent id: one"));
}
```

- [ ] **Step 2: Run the focused Rust tests and verify RED**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml public_relay_agents
```

Expected: compilation fails because the module and loader do not exist.

- [ ] **Step 3: Implement the minimal Rust registry module**

Define:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicRelayAgentRegistration {
    pub id: String,
    pub name: String,
    pub pubkey: String,
    pub channel_ids: Vec<String>,
    pub state: PublicRelayAgentState,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PublicRelayAgentState {
    Provisioning,
    Active,
    Failed,
}
```

The loader must:

- return an empty vector only when the file does not exist;
- require `version == 1`;
- trim IDs/names, lowercase and validate 64-character hex pubkeys;
- require non-empty, unique IDs, pubkeys, and Channel lists;
- de-duplicate Channel IDs while preserving order;
- reject unknown states through serde;
- never create or rewrite the file.

Resolve the command path through `app.path().app_data_dir()?.join("agents/public-relay-agents.json")`. Register the module and command in `lib.rs`.

- [ ] **Step 4: Run Rust tests and verify GREEN**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml public_relay_agents
```

Expected: all `public_relay_agents` tests pass.

- [ ] **Step 5: Add the TypeScript bridge and query**

Add the matching camel-case type to `types.ts`, call `invokeTauri("list_public_relay_agents")` in `tauri.ts`, and add:

```ts
export const publicRelayAgentsQueryKey = ["public-relay-agents"] as const;

export function usePublicRelayAgentsQuery(options?: { enabled?: boolean }) {
  return useQuery({
    enabled: options?.enabled ?? true,
    queryKey: publicRelayAgentsQueryKey,
    queryFn: listPublicRelayAgents,
    staleTime: 0,
    refetchOnWindowFocus: true,
  });
}
```

- [ ] **Step 6: Run formatter, typecheck, and focused Rust tests**

Run:

```bash
. ./bin/activate-hermit
cargo fmt --manifest-path desktop/src-tauri/Cargo.toml --check
cd desktop
pnpm typecheck
pnpm test -- --test-name-pattern public
```

Expected: commands exit zero.

- [ ] **Step 7: Commit the backend reader**

```bash
git add desktop/src-tauri/src/public_relay_agents.rs desktop/src-tauri/src/lib.rs desktop/src/shared/api/types.ts desktop/src/shared/api/tauri.ts desktop/src/features/agents/hooks.ts
git commit -s -m "feat(desktop): read public relay agent registry"
```

---

### Task 2: Merge Registered Members into the Desktop Agents View

**Files:**
- Modify: `desktop/src/features/agents/lib/externalRelayAgents.test.mjs`
- Modify: `desktop/src/features/agents/lib/externalRelayAgents.ts`
- Modify: `desktop/src/features/agents/ui/useManagedAgentActions.ts`
- Modify: `desktop/src/features/agents/ui/ExternalRelayAgentsSection.tsx`
- Preserve and integrate: `desktop/src/features/agents/ui/AgentsView.tsx`

**Interfaces:**
- Consumes: `PublicRelayAgentRegistration[]` from Task 1.
- Changes: `buildChannelAgentFallbacks({ channels, membersByChannelId, presence, registrations })`.
- Produces: `RelayAgent[]` where registered `member` identities are included only for matching registry and live Channel membership.

- [ ] **Step 1: Add failing behavior tests**

Add literal cases proving:

```js
test("registered member is an agent only in an explicitly registered shared channel", () => {
  const result = buildChannelAgentFallbacks({
    channels: [
      { id: "allowed", name: "Allowed" },
      { id: "other", name: "Other" },
    ],
    membersByChannelId: {
      allowed: [{ pubkey: "a".repeat(64), displayName: "Relay profile", role: "member", isAgent: false }],
      other: [{ pubkey: "a".repeat(64), displayName: "Relay profile", role: "member", isAgent: false }],
    },
    registrations: [{
      id: "research",
      name: "Research Agent",
      pubkey: "a".repeat(64),
      channelIds: ["allowed"],
      state: "active",
      enabled: true,
    }],
    presence: { ["a".repeat(64)]: "online" },
  });
  assert.deepEqual(result.map(({ name, channelIds }) => ({ name, channelIds })), [
    { name: "Research Agent", channelIds: ["allowed"] },
  ]);
});

test("unregistered member remains a person and failed registration is offline", () => {
  const failedPubkey = "b".repeat(64);
  const humanPubkey = "c".repeat(64);
  const result = buildChannelAgentFallbacks({
    channels: [{ id: "allowed", name: "Allowed" }],
    membersByChannelId: {
      allowed: [
        { pubkey: failedPubkey, displayName: "Old name", role: "member", isAgent: false },
        { pubkey: humanPubkey, displayName: "Human", role: "member", isAgent: false },
      ],
    },
    registrations: [{
      id: "failed",
      name: "Failed Agent",
      pubkey: failedPubkey,
      channelIds: ["allowed"],
      state: "failed",
      enabled: true,
    }],
    presence: { [failedPubkey]: "online", [humanPubkey]: "online" },
  });
  assert.deepEqual(
    result.map(({ name, pubkey, status }) => ({ name, pubkey, status })),
    [{ name: "Failed Agent", pubkey: failedPubkey, status: "offline" }],
  );
});
```

Also retain the existing bot fallback, kind `10100` de-duplication, local-managed exclusion, access filtering, and sort tests.

- [ ] **Step 2: Run the focused JS test and verify RED**

Run:

```bash
cd desktop
pnpm test -- src/features/agents/lib/externalRelayAgents.test.mjs
```

Expected: failure because `registrations` is ignored and `member` is filtered out.

- [ ] **Step 3: Implement minimal registration-aware merging**

Build a normalized registration map. For each Channel member:

- retain the existing `role === "bot" || isAgent` fallback;
- otherwise require an enabled registration in `active` or `failed`;
- require the registration `channelIds` to contain the current Channel;
- use the registration name as the authoritative local display name;
- force `failed` status to `offline`;
- keep `respondTo: null` because registry membership does not invent Relay policy.

In `useManagedAgentActions`:

- call `usePublicRelayAgentsQuery`;
- include registered pubkeys relevant to shared Channels in the presence query;
- pass registrations to the fallback builder;
- include registry query failures in the Relay Agents error aggregation;
- refetch the registry alongside Relay Agent refresh.

- [ ] **Step 4: Run the focused JS test and verify GREEN**

Run:

```bash
cd desktop
pnpm test -- src/features/agents/lib/externalRelayAgents.test.mjs
```

Expected: all focused tests pass.

- [ ] **Step 5: Refine the Relay Agent card state copy**

Render `failed` registrations as offline through the resulting `RelayAgent`. Keep the card read-only and do not add start/stop controls owned by the external Runner.

- [ ] **Step 6: Run Desktop tests and typecheck**

Run:

```bash
cd desktop
pnpm test -- src/features/agents/lib/externalRelayAgents.test.mjs src/features/agents/lib/agentAutocompleteEligibility.test.mjs
pnpm typecheck
pnpm check
```

Expected: all commands exit zero.

- [ ] **Step 7: Commit the Desktop merge**

```bash
git add desktop/src/features/agents/lib/externalRelayAgents.test.mjs desktop/src/features/agents/lib/externalRelayAgents.ts desktop/src/features/agents/ui/useManagedAgentActions.ts desktop/src/features/agents/ui/ExternalRelayAgentsSection.tsx desktop/src/features/agents/ui/AgentsView.tsx
git commit -s -m "feat(desktop): show registered relay members as agents"
```

---

### Task 3: Deployment Registry Core and Atomic Desktop Projection

**Files:**
- Create: `platform/lib/public-agent-registry.mjs`
- Create: `platform/tests/public-agent-registry.test.mjs`
- Create: `platform/agents/public-agents.json`

**Interfaces:**
- Produces: `loadRegistry(path)`, `validateRegistry(value)`, `upsertAgent(registry, agent)`, `projectDesktopRegistry(registry)`, `writeJsonAtomic(path, value)`, and `syncDesktopRegistry({ registryPath, desktopPath })`.

- [ ] **Step 1: Write failing Node registry tests**

Use `node:test` and temporary directories. Cover:

- missing source registry returns `{ version: 1, agents: [] }`;
- duplicate ID, duplicate pubkey, empty Channels, invalid state, and secret-like fields are rejected;
- identical upsert is idempotent and conflicting upsert fails;
- Desktop projection omits `configPath`, `workdir`, `source`, model, and prompt path;
- atomic sync leaves valid JSON and mode `0600`.

Example assertion:

```js
assert.deepEqual(projectDesktopRegistry(registry), {
  version: 1,
  agents: [{
    id: "product",
    name: "Product Agent",
    pubkey: "a".repeat(64),
    channelIds: ["channel-a"],
    state: "active",
    enabled: true,
  }],
});
```

- [ ] **Step 2: Run registry tests and verify RED**

Run:

```bash
node --test platform/tests/public-agent-registry.test.mjs
```

Expected: module-not-found failure.

- [ ] **Step 3: Implement the registry core**

Use Node built-ins only. `writeJsonAtomic` must create the parent directory, write a mode-`0600` temporary file in that directory, `fsync`, rename, and leave no temp file. Reject any object key matching `/private|secret|token|api.?key|nsec/i`.

- [ ] **Step 4: Run registry tests and verify GREEN**

Run:

```bash
node --test platform/tests/public-agent-registry.test.mjs
```

Expected: all registry tests pass.

- [ ] **Step 5: Create the initial versioned empty registry**

Write:

```json
{
  "version": 1,
  "agents": []
}
```

The live five-Agent migration occurs only in Task 6 after dynamic supervisor tests pass.

- [ ] **Step 6: Record local verification**

Because `platform/` is outside `vendor/buzz` Git, do not stage these files in the Buzz commit. Preserve the test command and output in the final delivery evidence.

---

### Task 4: Idempotent Create and Resume Commands

**Files:**
- Create: `platform/lib/public-agent-provisioner.mjs`
- Create: `platform/bin/create-public-agent`
- Create: `platform/bin/resume-public-agent`
- Create: `platform/tests/public-agent-provisioner.test.mjs`

**Interfaces:**
- Produces CLI contract from the approved Spec.
- Consumes the Task 3 registry core.
- Executes external tools through an injected `run(command, args, options)` boundary so tests exercise real filesystem behavior with deterministic fake Relay commands.

- [ ] **Step 1: Write failing provisioner tests**

Use a temporary PoC root and executable fake `buzz`, `buzz-local`, and `start-agent` commands. Test:

1. no `--channel` exits `2` and creates nothing;
2. invalid Channel fails before identity generation;
3. protected or out-of-root workdirs and system prompt files fail before identity generation;
4. successful create writes a `0600` identity, non-secret Agent config, workdir, `provisioning -> active` registry transition, member-role calls, Desktop projection, and start call;
5. only repeated `--channel` values are joined;
6. second Channel failure removes the first Channel and cleans local artifacts;
7. Runner failure retains identity/membership and writes `state: "failed"`;
8. identical rerun and `resume-public-agent --id` do not rotate identity.

- [ ] **Step 2: Run provisioner tests and verify RED**

Run:

```bash
node --test platform/tests/public-agent-provisioner.test.mjs
```

Expected: module-not-found failure.

- [ ] **Step 3: Implement argument parsing and preflight validation**

Accept:

```text
--id <slug>
--name <display-name>
--channel <uuid>  # repeatable and required
--model <model>   # optional
--workdir <path>  # optional
--system-prompt-file <path>  # optional
```

Validate every Channel with owner credentials using `buzz channels get --channel <uuid>` before `buzz-admin generate-key`. Normalize paths and reject the home directory, `/`, the PoC root, all paths under `platform/`, and every workdir outside the PoC. System prompt files must also remain inside the PoC and outside `platform/env/`.

- [ ] **Step 4: Implement provisioning and rollback**

- Generate the identity with `buzz-local run --rm --no-deps --entrypoint /usr/local/bin/buzz-admin relay generate-key`.
- Keep the private key only in `platform/env/identities/<id>.env` mode `0600`.
- Write `platform/agents/<id>/agent.env` without the private key; `start-agent` sources the identity separately.
- Register the Relay member with `buzz-admin add-member --role member`.
- Add each Channel member using the owner identity and `buzz channels add-member --role member`.
- Roll back partial Channel additions with `buzz channels remove-member`.
- On pre-Runner failure remove the newly added Relay member only when this invocation created it.
- On Runner failure retain identity, configuration and memberships, then mark `failed`.
- On success verify the tmux pane and Relay membership before marking `active`.

- [ ] **Step 5: Implement resume semantics**

`resume-public-agent --id <id>` must require an existing `failed` or `active` registry record, verify identity/config/workdir/Channel membership, repair missing `member` memberships, start the Runner, and set `active` only after verification.

- [ ] **Step 6: Run provisioner tests and verify GREEN**

Run:

```bash
node --test platform/tests/public-agent-provisioner.test.mjs
```

Expected: all provisioner tests pass with no secret values in captured output.

- [ ] **Step 7: Verify executable modes and help output**

Run:

```bash
chmod 700 platform/bin/create-public-agent platform/bin/resume-public-agent
platform/bin/create-public-agent --help
platform/bin/resume-public-agent --help
```

Expected: help exits zero; scripts are owner-executable and do not print credentials.

---

### Task 5: Registry-Driven Runner Supervisor

**Files:**
- Create: `platform/bin/list-public-agent-ids`
- Modify: `platform/bin/agents-up`
- Modify: `platform/bin/agents-status`
- Modify: `platform/bin/start-agent`
- Modify: `platform/tests/agent-role-scripts.sh`
- Create: `platform/tests/public-agent-supervisor.test.mjs`

**Interfaces:**
- Produces: `list-public-agent-ids [--startable]`.
- Changes: `start-agent <id>` resolves `configPath`, `workdir`, and identity path from the registry.
- Changes: `agents-up` and `agents-status` enumerate registry records.

- [ ] **Step 1: Write failing supervisor behavior tests**

Run copied scripts inside a temporary PoC with a fixture registry containing one enabled active Agent, one enabled failed Agent, and one disabled Agent. Fake tmux and buzz-acp. Assert:

- `list-public-agent-ids --startable` returns active and failed enabled IDs only;
- `start-agent research` sources the configured identity separately and launches in the configured workdir;
- an ID absent from the registry exits `2`;
- `agents-up` creates windows only for startable IDs;
- `agents-status` reports each registered startable ID and returns failure when a pane is dead.

- [ ] **Step 2: Run supervisor tests and verify RED**

Run:

```bash
node --test platform/tests/public-agent-supervisor.test.mjs
```

Expected: failure because the list helper is absent and scripts are hardcoded.

- [ ] **Step 3: Implement registry discovery**

`list-public-agent-ids` imports the registry core, validates the source registry, and prints one validated slug per line. Shell callers read with `while IFS= read -r id`; no whitespace-split `ROLES` variable remains.

- [ ] **Step 4: Refactor start and status scripts**

`start-agent` must:

- resolve registry metadata without `eval`;
- require the identity/config/workdir;
- source config and identity with automatic export;
- unset transient Claude Provider overrides exactly as the existing script does;
- launch the existing `platform/bin/buzz-acp`.

`agents-up` and `agents-status` must preserve `buzz-local-agents` and per-ID tmux windows.

- [ ] **Step 5: Run supervisor and existing role tests**

Run:

```bash
node --test platform/tests/public-agent-supervisor.test.mjs
bash platform/tests/agent-role-scripts.sh
```

Expected: all tests pass.

- [ ] **Step 6: Run shell syntax checks**

Run:

```bash
bash -n platform/bin/agents-up platform/bin/agents-status platform/bin/agents-down platform/bin/start-agent platform/bin/list-public-agent-ids
```

Expected: exit zero.

---

### Task 6: Migrate the Existing Five Agents Without Identity Drift

**Files:**
- Create: `platform/bin/migrate-public-agents`
- Create: `platform/tests/migrate-public-agents.test.mjs`
- Modify at runtime: `platform/agents/public-agents.json`
- Create at runtime: Desktop projection in the Buzz app-data directory

**Interfaces:**
- Produces: `migrate-public-agents --dry-run` and `migrate-public-agents --apply`.
- Consumes existing identity files, Agent configs, worktrees, and the two approved Channel UUIDs.

- [ ] **Step 1: Write a failing migration fixture test**

Fixture five identities and directories. Assert dry-run performs no writes; apply creates five `source: "builtin"` records using existing public keys, paths, and exactly:

```text
c1829c53-70f5-446d-b4d5-305d4a76088a
304753ff-3464-4389-a29b-de80766d921b
```

Assert no identity file content or modification time changes.

- [ ] **Step 2: Run migration test and verify RED**

Run:

```bash
node --test platform/tests/migrate-public-agents.test.mjs
```

Expected: module or command missing.

- [ ] **Step 3: Implement migration and collision checks**

Read public keys only. Refuse if an ID maps to a different public key, a public key maps to a different ID, a required worktree/config/identity is missing, or either Channel is absent. Project the sanitized Desktop registry atomically.

- [ ] **Step 4: Run migration test and verify GREEN**

Run:

```bash
node --test platform/tests/migrate-public-agents.test.mjs
```

Expected: all tests pass.

- [ ] **Step 5: Snapshot live identity and Runner state**

Capture only non-secret data:

```bash
for id in product tech coding cr qa; do
  awk -F= '/^PUBLIC_KEY=/ {print FILENAME ":" $2}' "platform/env/identities/$id.env"
done
platform/bin/agents-status
```

Do not print identity private keys or Agent environment files.

- [ ] **Step 6: Apply the live migration**

Run:

```bash
platform/bin/migrate-public-agents --dry-run
platform/bin/migrate-public-agents --apply
```

Then compare the five public keys and workdir paths with the pre-migration snapshot.

- [ ] **Step 7: Restart through the dynamic supervisor**

Run:

```bash
platform/bin/agents-down
platform/bin/agents-up
platform/bin/agents-status
```

Expected: all five registered windows are running with unchanged IDs and public keys.

---

### Task 7: Full Verification, Desktop Build, Installation, and Live Acceptance

**Files:**
- Modify only if a verified failure requires a regression fix.
- Build artifact: `desktop/src-tauri/target/release/bundle/macos/Buzz.app` or the repository-defined equivalent.
- Install target: `/Applications/Buzz.app`.

**Interfaces:**
- Consumes all prior tasks.
- Produces the locally installed v0.5.0 Desktop and live acceptance evidence.

- [ ] **Step 1: Run all platform tests**

Run:

```bash
node --test platform/tests/public-agent-registry.test.mjs platform/tests/public-agent-provisioner.test.mjs platform/tests/public-agent-supervisor.test.mjs platform/tests/migrate-public-agents.test.mjs
bash platform/tests/agent-role-scripts.sh
```

Expected: zero failures.

- [ ] **Step 2: Run Desktop focused and full gates**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml public_relay_agents
cd desktop
pnpm test
pnpm typecheck
pnpm check
pnpm build
```

Expected: all commands exit zero.

- [ ] **Step 3: Scan registries and output for secrets**

Use a scanner that reports keys and paths, never values. Fail on fields matching `private`, `secret`, `token`, `apiKey`, `nsec`, `BUZZ_PRIVATE_KEY`, or `ANTHROPIC_API_KEY` in either registry.

- [ ] **Step 4: Build the Desktop application**

Run the repository-supported macOS Tauri build from the Hermit environment:

```bash
cd desktop
pnpm tauri:build
```

Expected: a Buzz v0.5.0 `.app` bundle is produced.

- [ ] **Step 5: Install without a restart loop**

Before replacing `/Applications/Buzz.app`, stop the application once, copy the verified bundle, apply the same local ad-hoc signing method used by the current installation, then launch once. Confirm there is only one Buzz process and no LaunchAgent repeatedly relaunching it.

- [ ] **Step 6: Verify the Desktop registry and UI**

Confirm the runtime projection has five sanitized entries. Open Agents and verify Product, Tech, Coding, CR, and QA are shown in Relay agents; inspect each card’s Channel count and online state.

- [ ] **Step 7: Verify Relay and Channel compatibility**

For both approved Channels:

- query current members and confirm all five public keys have role `member`;
- Mention one Agent from the local Desktop and receive a reply;
- retain the previously verified official v0.5.0 LAN-client compatibility by making no `bot` role change.

- [ ] **Step 8: Exercise future registration without persisting a test Agent**

Run no-side-effect negative cases:

```bash
platform/bin/create-public-agent --id missing-channel --name "Missing Channel"
platform/bin/create-public-agent --id invalid-channel --name "Invalid Channel" --channel 00000000-0000-0000-0000-000000000000
```

Expected: both fail before identity creation. Do not create a live sixth Agent unless the user supplies its intended name and Channel.

- [ ] **Step 9: Review and commit final Desktop changes**

Run `git diff --check`, inspect all staged paths, and commit only reviewed Buzz files:

```bash
git add desktop docs/superpowers/plans/2026-07-29-public-relay-agent-registry.md
git commit -s -m "feat: register public relay agents"
```

Do not stage unrelated files.
