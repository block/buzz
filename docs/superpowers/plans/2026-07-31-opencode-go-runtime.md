# OpenCode Go Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Promote OpenCode to a first-class Buzz ACP runtime that launches with `opencode acp`, exposes OpenCode's model catalog, and offers the ACP-advertised account connection without making authentication a readiness requirement.

**Architecture:** `KnownAcpRuntime` remains the single source of runtime capability facts. A new `supports_account_connection` capability is serialized through `AcpRuntimeCatalogEntry`, mapped into the frontend, and consumed by a pure settings helper so the React surface never checks the `opencode` runtime ID. Existing ACP auth and model-discovery bridges remain unchanged and receive `opencode acp`.

**Tech Stack:** Rust 2021, Tauri 2, React 19, TypeScript, Node test runner, Cargo tests, Biome.

## Global Constraints

- Do not add dependencies.
- Do not store, parse, copy, or display OpenCode credentials.
- Do not add OpenCode Go as a direct bundled Buzz Agent provider.
- Do not hard-code OpenCode model IDs; preserve provider-qualified IDs returned by ACP.
- Runtime capability facts originate in `KnownAcpRuntime`, not React components.
- Keep OpenCode authentication optional for readiness because unauthenticated free models remain usable.
- Keep changes minimal; do not refactor unrelated harness catalog code.
- Every commit includes `Co-authored-by: Ralf <wunder.kontakt@posteo.de>` before `Signed-off-by: Ralf <wunder.kontakt@posteo.de>`.

---

### Task 1: Promote OpenCode into the authoritative runtime catalog

**Files:**

- Modify: `desktop/src-tauri/src/managed_agents/discovery/runtime_metadata.rs`
- Modify: `desktop/src-tauri/src/managed_agents/discovery.rs`
- Modify: `desktop/src-tauri/src/managed_agents/discovery/presets.rs`
- Modify: `desktop/src-tauri/src/managed_agents/types.rs`
- Modify: `desktop/src-tauri/src/commands/agent_discovery.rs`
- Modify: every test-only `KnownAcpRuntime` literal reported by `rg -n 'KnownAcpRuntime \\{' desktop/src-tauri/src`
- Test: `desktop/src-tauri/src/managed_agents/discovery/tests.rs`
- Test: `desktop/src-tauri/src/managed_agents/discovery/runtime_metadata.rs`

**Interfaces:**

- Consumes: existing `KnownAcpRuntime`, `normalize_agent_args`, `discover_acp_runtimes_from`, and `AcpRuntimeCatalogEntry`.
- Produces: `KnownAcpRuntime::supports_account_connection: bool` and `AcpRuntimeCatalogEntry::supports_account_connection: bool`; one built-in runtime with ID `opencode`, command `opencode`, and default args `["acp"]`.

- [ ] **Step 1: Add failing runtime and catalog tests**

Add assertions equivalent to:

```rust
#[test]
fn opencode_uses_native_acp_command_and_account_connection() {
    assert_eq!(
        normalize_agent_args("opencode", Vec::new()),
        vec!["acp".to_string()]
    );

    let runtime = known_acp_runtime_exact("opencode").expect("OpenCode runtime");
    assert_eq!(runtime.commands, &["opencode"]);
    assert!(runtime.supports_acp_model_switching);
    assert!(runtime.supports_account_connection);
    assert!(runtime.model_env_var.is_none());
    assert!(runtime.provider_env_var.is_none());
}
```

Extend the catalog test that uses an injected executable resolver so it asserts
that exactly one entry has ID `opencode`, its source is `HarnessSource::Builtin`,
its command is `opencode`, its default args are `["acp"]`, and
`supports_account_connection` is true.

- [ ] **Step 2: Run the focused Rust tests and confirm the red state**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-desktop managed_agents::discovery
```

Expected: compilation or assertions fail because OpenCode is still a preset and
the capability fields do not exist.

- [ ] **Step 3: Add the runtime capability fields**

Add to `KnownAcpRuntime`:

```rust
/// Whether settings may offer the runtime's ACP-advertised account connection
/// even when authentication is not a readiness requirement.
pub supports_account_connection: bool,
```

Add to `AcpRuntimeCatalogEntry`:

```rust
/// Whether the runtime exposes an optional ACP account-connection flow.
pub supports_account_connection: bool,
```

Set the field from `runtime.supports_account_connection` in the built-in
catalog projection. Set it to `false` in preset and custom constructors and in
test fixtures unless the fixture specifically represents OpenCode.

- [ ] **Step 4: Move OpenCode from preset to built-in metadata**

Remove the `PresetHarness { id: "opencode", ... }` entry. Add a
`KnownAcpRuntime` entry with these exact behavioral values:

```rust
KnownAcpRuntime {
    id: "opencode",
    label: "OpenCode",
    commands: &["opencode"],
    aliases: &[],
    mcp_command: None,
    mcp_hooks: false,
    underlying_cli: None,
    cli_install_commands: &[],
    cli_install_commands_windows: &[],
    adapter_install_commands: &[],
    cli_install_instructions_url: "https://opencode.ai/docs/",
    adapter_install_instructions_url: "",
    cli_install_hint: "Buzz talks to OpenCode through the OpenCode CLI's ACP mode.",
    adapter_install_hint: "",
    skill_dir: None,
    supports_acp_model_switching: true,
    supports_account_connection: true,
    model_env_var: None,
    provider_env_var: None,
    provider_locked: false,
    default_env: &[],
    config_file_path: Some("~/.config/opencode/opencode.json"),
    config_file_format: Some("json"),
    supports_acp_native_config: false,
    thinking_env_var: None,
    max_tokens_env_var: None,
    context_limit_env_var: None,
    required_normalized_fields: &[],
    login_hint: None,
    auth_probe_args: None,
    // Existing avatar_url field uses the bundled-ID icon path in the frontend;
    // keep the backend URL empty instead of introducing a remote asset.
    avatar_url: "",
}
```

Extend `default_agent_args` so `"opencode"` returns
`Some(vec!["acp".to_string()])`.

- [ ] **Step 5: Run the focused Rust tests and confirm the green state**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-desktop managed_agents::discovery
```

Expected: all discovery tests pass.

- [ ] **Step 6: Commit the backend catalog change**

```bash
git add desktop/src-tauri/src/managed_agents/discovery.rs \
  desktop/src-tauri/src/managed_agents/discovery \
  desktop/src-tauri/src/managed_agents/types.rs \
  desktop/src-tauri/src/commands/agent_discovery.rs
git commit -m "feat(desktop): promote OpenCode runtime" \
  -m "Co-authored-by: Ralf <wunder.kontakt@posteo.de>" -s
```

---

### Task 2: Expose optional account connection in the settings UI

**Files:**

- Modify: `desktop/src/shared/api/tauri.ts`
- Modify: `desktop/src/shared/api/types.ts`
- Test: `desktop/src/shared/api/tauri.test.mjs`
- Modify: `desktop/src/features/settings/ui/harnessCatalogLogic.ts`
- Modify: `desktop/src/features/settings/ui/harnessCatalogLogic.test.mjs`
- Modify: `desktop/src/features/settings/ui/HarnessRow.tsx`
- Modify: `desktop/src/testing/e2eBridge.ts`

**Interfaces:**

- Consumes: backend JSON field `supports_account_connection`.
- Produces: frontend property `supportsAccountConnection: boolean` and pure function `canConnectRuntimeAccount(entry: AcpRuntimeCatalogEntry): boolean`.

- [ ] **Step 1: Add failing frontend mapping and behavior tests**

Extend the catalog-entry factory with:

```js
supportsAccountConnection: false,
```

Import `canConnectRuntimeAccount` and add:

```js
describe("canConnectRuntimeAccount", () => {
  it("connects a mandatory logged-out runtime", () => {
    assert.equal(
      canConnectRuntimeAccount(
        entry({
          availability: "available",
          authStatus: { status: "logged_out" },
        }),
      ),
      true,
    );
  });

  it("connects an optional account-capable runtime without blocking readiness", () => {
    assert.equal(
      canConnectRuntimeAccount(
        entry({
          availability: "available",
          authStatus: { status: "not_applicable" },
          supportsAccountConnection: true,
        }),
      ),
      true,
    );
  });

  it("does not connect unavailable or non-capable runtimes", () => {
    assert.equal(
      canConnectRuntimeAccount(
        entry({
          availability: "not_installed",
          supportsAccountConnection: true,
        }),
      ),
      false,
    );
    assert.equal(
      canConnectRuntimeAccount(entry({ availability: "available" })),
      false,
    );
  });
});
```

Add a raw-mapping assertion that
`supports_account_connection: true` becomes
`supportsAccountConnection: true`.

- [ ] **Step 2: Run the focused frontend tests and confirm the red state**

Run:

```bash
# From desktop/ after activating ../bin/activate-hermit:
node --import ./test-loader.mjs --experimental-strip-types --test \
  src/features/settings/ui/harnessCatalogLogic.test.mjs \
  src/shared/api/tauri.test.mjs
```

Expected: the import or assertions fail because the helper and mapped property
do not exist.

- [ ] **Step 3: Map the backend capability into TypeScript**

Add the optional raw field:

```ts
supports_account_connection?: boolean;
```

Add the required frontend field:

```ts
/** True when settings may offer an ACP account-connection action. */
supportsAccountConnection: boolean;
```

Map it with a backward-compatible default:

```ts
supportsAccountConnection: entry.supports_account_connection ?? false,
```

Add `supports_account_connection: false` to the E2E bridge's default raw runtime
objects or normalize it in `withMockRuntimeConfigMetadata`. OpenCode-specific
test fixtures set it to `true`.

- [ ] **Step 4: Implement and consume the pure connection predicate**

Add:

```ts
export function canConnectRuntimeAccount(
  entry: AcpRuntimeCatalogEntry,
): boolean {
  return (
    entry.availability === "available" &&
    (entry.authStatus.status === "logged_out" ||
      entry.supportsAccountConnection)
  );
}
```

Import the helper in `HarnessRow.tsx` and replace the inline
`availability === "available" && authStatus === "logged_out"` expression with:

```ts
const canConnectAccount = canConnectRuntimeAccount(runtime);
```

Keep onboarding readiness unchanged: optional OpenCode authentication must not
turn a usable runtime into a blocked one.

- [ ] **Step 5: Run frontend unit tests and typecheck**

Run:

```bash
. ./bin/activate-hermit
pnpm --dir desktop test
pnpm --dir desktop typecheck
```

Expected: all frontend unit tests and TypeScript checks pass.

- [ ] **Step 6: Commit the frontend capability change**

```bash
git add desktop/src/shared/api/tauri.ts \
  desktop/src/shared/api/tauri.test.mjs \
  desktop/src/shared/api/types.ts \
  desktop/src/features/settings/ui/HarnessRow.tsx \
  desktop/src/features/settings/ui/harnessCatalogLogic.ts \
  desktop/src/features/settings/ui/harnessCatalogLogic.test.mjs \
  desktop/src/testing/e2eBridge.ts
git commit -m "feat(desktop): connect optional OpenCode account" \
  -m "Co-authored-by: Ralf <wunder.kontakt@posteo.de>" -s
```

---

### Task 3: Verify the ACP-advertised OpenCode login command

**Files:**

- Test: `desktop/src-tauri/src/commands/agent_auth.rs`

**Interfaces:**

- Consumes: existing `adapter_terminal_argv(runtime_label, method, fallback_command)`.
- Produces: regression coverage proving Buzz executes OpenCode's advertised terminal-auth command without constructing or storing credentials.

- [ ] **Step 1: Add the OpenCode terminal-auth regression test**

Inside the existing `agent_auth.rs` test module, add:

```rust
#[test]
fn opencode_terminal_auth_uses_advertised_command() {
    let method = AcpAuthMethod {
        id: "opencode-login".to_string(),
        name: "OpenCode login".to_string(),
        description: None,
        method_type: Some("terminal".to_string()),
        args: Vec::new(),
        command: Vec::new(),
        meta: Some(serde_json::json!({
            "terminal-auth": {
                "command": "opencode",
                "args": ["auth", "login"]
            }
        })),
    };

    assert_eq!(
        adapter_terminal_argv("OpenCode", &method, "fallback").expect("argv"),
        vec!["opencode", "auth", "login"]
    );
}
```

If command resolution returns an absolute path in the test environment, compare
`Path::new(&argv[0]).file_name()` with `Some(OsStr::new("opencode"))` and compare
`&argv[1..]` with `["auth", "login"]`.

- [ ] **Step 2: Run the focused auth test**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-desktop opencode_terminal_auth_uses_advertised_command
```

Expected: PASS using the existing generic terminal-auth implementation; no
production auth code is added.

- [ ] **Step 3: Commit the auth regression coverage**

```bash
git add desktop/src-tauri/src/commands/agent_auth.rs
git commit -m "test(desktop): cover OpenCode ACP login command" \
  -m "Co-authored-by: Ralf <wunder.kontakt@posteo.de>" -s
```

---

### Task 4: Run complete verification and prepare review

**Files:**

- Review: all files changed since `main`
- Update only if behavior rules changed: `desktop/src/features/agents/AGENTS.md`

**Interfaces:**

- Consumes: Tasks 1-3.
- Produces: a clean, fully tested feature branch ready for pull request review.

- [ ] **Step 1: Run formatters on touched code**

Run:

```bash
. ./bin/activate-hermit
cargo fmt --all
pnpm --dir desktop format
```

Inspect `git status` immediately and revert no unrelated formatter changes;
retain only files in this plan.

- [ ] **Step 2: Run the complete affected-package suites**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-desktop
pnpm --dir desktop test
pnpm --dir desktop typecheck
pnpm --dir desktop check
```

Expected: every command exits 0.

- [ ] **Step 3: Run the repository CI gate**

Run:

```bash
. ./bin/activate-hermit
just ci
```

Expected: format, Clippy, desktop lint, unit tests, and builds all pass. If the
environment lacks an existing tool or service, report the exact command and
error; do not claim the gate passed.

- [ ] **Step 4: Self-review the exact branch state**

Run:

```bash
git diff main...HEAD --check
git diff main...HEAD
git status --short
git rev-parse HEAD
```

Check for duplicate OpenCode catalog rows, debug output, secret handling,
hard-coded model IDs, hard-coded runtime-ID checks in React, and missing
constructor fields. Confirm `desktop/src/features/agents/AGENTS.md` remains
accurate; if no rule changed, record “no agent-config rules changed” in the PR.

- [ ] **Step 5: Verify commit trailers**

Run:

```bash
git log --format=full main..HEAD
```

Expected: every commit contains both:

```text
Co-authored-by: Ralf <wunder.kontakt@posteo.de>
Signed-off-by: Ralf <wunder.kontakt@posteo.de>
```

- [ ] **Step 6: Push and open the channel-linked pull request**

Push the feature branch, then run:

```bash
buzz pr open --help
buzz pr open --channel c0617041-4eca-5168-8e65-a8ca096b4e0e
```

Use the discovered CLI flags for repository, base `main`, head
`codex/opencode-go-runtime`, title, and a body containing the implementation
summary, exact checks, known limitations, and “no agent-config rules changed.”
