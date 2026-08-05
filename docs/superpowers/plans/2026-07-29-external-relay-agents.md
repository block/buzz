# External Relay Agents Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Display invocable external Relay agents in a separate read-only section of the v0.5.0 Desktop Agents page.

**Architecture:** Derive external agents from the existing Relay agent query with a pure normalized selector, excluding local managed identities. Render the result with a focused read-only card section that only opens existing profile panels.

**Tech Stack:** React 19, TypeScript, TanStack Query, Tauri v2, Node test runner, Tailwind CSS.

## Global Constraints

- Keep the application version at v0.5.0.
- Do not persist Relay agents into the Desktop managed-agent store.
- Do not introduce runtime-control actions for external agents.
- Reuse current Relay sharing and allowlist eligibility semantics.
- Do not expose private keys, environment variables, external logs, or work directories.

---

### Task 1: Select visible external Relay agents

**Files:**
- Create: `desktop/src/features/agents/lib/externalRelayAgents.ts`
- Create: `desktop/src/features/agents/lib/externalRelayAgents.test.mjs`

**Interfaces:**
- Consumes: `RelayAgent`, `relayAgentIsSharedWithUser`, and normalized managed pubkeys.
- Produces:

```ts
export function selectVisibleExternalRelayAgents(input: {
  currentPubkey?: string | null;
  managedAgentPubkeys: Iterable<string>;
  relayAgents: readonly RelayAgent[] | undefined;
  sharedChannelIds: ReadonlySet<string>;
}): RelayAgent[]
```

- [ ] **Step 1: Write the failing selector test**

Create fixtures for one local managed agent, Product/Tech/Coding/CR/QA as eligible Relay agents, one unshared agent, and a duplicate uppercase pubkey. Assert that only the five eligible external agents remain and are sorted by status then name.

- [ ] **Step 2: Run the selector test and verify RED**

Run:

```bash
node --import ./test-loader.mjs --experimental-strip-types --test src/features/agents/lib/externalRelayAgents.test.mjs
```

Expected: failure because `externalRelayAgents.ts` does not exist.

- [ ] **Step 3: Implement the selector**

Normalize managed and Relay pubkeys, reuse `relayAgentIsSharedWithUser`, retain the first normalized Relay identity, and apply deterministic status/name sorting.

- [ ] **Step 4: Run the selector test and verify GREEN**

Run the Step 2 command.

Expected: all selector tests pass.

### Task 2: Render the read-only Relay section

**Files:**
- Create: `desktop/src/features/agents/ui/ExternalRelayAgentsSection.tsx`
- Modify: `desktop/src/features/agents/ui/useManagedAgentActions.ts`
- Modify: `desktop/src/features/agents/ui/AgentsView.tsx`

**Interfaces:**
- Consumes: `RelayAgent[]`, loading/error state, and `onOpenProfile(pubkey)`.
- Produces:

```ts
export function ExternalRelayAgentsSection(props: {
  agents: readonly RelayAgent[];
  error: Error | null;
  isLoading: boolean;
  onOpenProfile: (pubkey: string) => void;
}): React.ReactNode
```

- [ ] **Step 1: Extend `useManagedAgentActions`**

Read the current identity, calculate active shared channel IDs, call `selectVisibleExternalRelayAgents`, and return `externalRelayAgents` without modifying existing managed-agent actions.

- [ ] **Step 2: Add the read-only section component**

Render a heading, description, loading skeleton, inline error, and one `AgentIdentityCard` per external agent. Use the existing user-profile query for avatars and an informational status badge. Do not pass any action menu or runtime control.

- [ ] **Step 3: Insert the section into `AgentsView`**

Render it after `UnifiedAgentsSection` and before `TeamsSection`. Pass Relay query state and the existing profile-panel opener.

- [ ] **Step 4: Type-check**

Run:

```bash
pnpm typecheck
```

Expected: TypeScript exits 0.

### Task 3: Regression and release verification

**Files:**
- Verify all files changed in Tasks 1–2.

**Interfaces:**
- Consumes: completed selector and UI section.
- Produces: a verified v0.5.0 release app.

- [ ] **Step 1: Run focused tests**

```bash
node --import ./test-loader.mjs --experimental-strip-types --test \
  src/features/agents/lib/externalRelayAgents.test.mjs \
  src/features/agents/lib/agentAutocompleteEligibility.test.mjs
```

- [ ] **Step 2: Run the full Desktop test suite**

```bash
node --import ./test-loader.mjs --experimental-strip-types --test "src/**/*.test.mjs"
```

Expected: zero failures.

- [ ] **Step 3: Build the v0.5.0 app**

```bash
pnpm tauri build --features mesh-llm --target aarch64-apple-darwin --bundles app
```

Expected: `Buzz.app` is produced and `CFBundleShortVersionString` is `0.5.0`.

- [ ] **Step 4: Install with a recoverable backup**

Back up the current `/Applications/Buzz.app`, sign the local release with Hardened Runtime and the repository entitlements, install it, and run strict `codesign` verification.

- [ ] **Step 5: Verify runtime acceptance**

Open the Agents page and confirm Product, Tech, Coding, CR, and QA appear under `Relay agents`, with no local runtime controls. Recheck Relay health, five Runner processes, main-channel membership, and history.
