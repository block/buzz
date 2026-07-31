# OpenCode Go Runtime Integration

## Goal

Make OpenCode a fully supported Buzz agent runtime so a user with an OpenCode
Go subscription can:

1. select OpenCode in Buzz;
2. complete OpenCode's official login flow;
3. select a model exposed by the authenticated OpenCode CLI; and
4. run the agent through `opencode acp`.

Buzz must not implement OpenCode Go's API, billing, credential storage, or model
catalog itself.

## Current State

Buzz `main` contains OpenCode as a tier-2 preset. The preset makes the harness
visible, supplies `opencode acp`, and permits ACP model discovery. It has no
entry in the authoritative `KnownAcpRuntime` catalog, however, so it cannot
declare first-class runtime capabilities or use Buzz's ACP account-connection
bridge.

OpenCode's ACP implementation already advertises:

- the `opencode-login` authentication method;
- terminal-auth metadata for `opencode auth login`;
- a model configuration option containing provider-qualified model IDs; and
- ACP model switching.

## Design

### Runtime Catalog

Move OpenCode from `PRESET_HARNESSES` into `KNOWN_ACP_RUNTIMES`.

The built-in runtime entry will define:

- runtime ID and label: `opencode` / `OpenCode`;
- executable: `opencode`;
- default arguments: `acp`;
- no separate ACP adapter or underlying CLI;
- native ACP model switching;
- no provider, model, or thinking environment variables;
- no required normalized fields;
- optional ACP account connection;
- official OpenCode installation guidance; and
- no mandatory login-status probe.

`default_agent_args` will recognize `opencode` and return `["acp"]`. This keeps
runtime discovery, model discovery, readiness checks, spawn hashing, and actual
agent launch on the same argument normalization path.

Removing the preset entry prevents duplicate OpenCode rows in the runtime
catalog.

### Authentication

OpenCode supports unauthenticated free models, and its supported CLI versions
do not expose one stable, non-interactive login-status command. Buzz will
therefore treat OpenCode authentication as optional for readiness instead of
guessing status from credential files or command output.

Add a `supports_account_connection` capability to `KnownAcpRuntime` and project
it through `AcpRuntimeCatalogEntry`. OpenCode sets it to `true`. Settings uses
the capability, rather than a hard-coded runtime ID, to offer account
connection even when authentication is not a readiness requirement.

The connection action uses Buzz's existing ACP authentication bridge:

1. `buzz-acp auth-methods --json` starts `opencode acp`;
2. OpenCode advertises `opencode-login` and terminal-auth metadata;
3. Buzz opens the advertised `opencode auth login` command in a visible
   terminal; and
4. OpenCode stores and refreshes its own credentials; and
5. Buzz refreshes runtime and model discovery after the terminal flow starts.

Buzz will not read, copy, persist, or display the OpenCode API key. This also
keeps OpenCode Go and any other OpenCode account plans behind the same official
authentication contract.

### Model Discovery and Selection

Before an agent is created, Buzz will run its existing ACP model discovery
against `opencode acp`. OpenCode's stable model config option is normalized into
Buzz model options using the existing provider-qualified IDs.

The UI continues to show `OpenCode` as the runtime. OpenCode Go is represented
by the models that OpenCode exposes after the user's account is authenticated,
not by a separate hard-coded Buzz provider row.

When a model is selected, Buzz stores the provider-qualified model ID in its
existing normalized model field. The existing `BUZZ_ACP_MODEL` startup path
applies the selection through ACP. Live model changes continue through the
existing ACP config-option/session-model mechanisms.

### Errors

Existing runtime states remain authoritative:

- missing `opencode`: show the official installation guidance;
- optional account connection: keep the runtime ready while exposing the
  connection action in settings;
- authentication required during model discovery or start: show the existing
  sign-in guidance without reading OpenCode's credential files;
- successful discovery with no models: retain the existing warning and retry
  behavior when the screen is reopened;
- ACP command failure: surface the redacted subprocess error through the
  existing discovery and start paths.

No OpenCode-specific secret parsing or fallback login command will be added.
The terminal command advertised by OpenCode remains the source of truth.

## Testing

Add or update tests that verify:

- `opencode` resolves as a built-in `KnownAcpRuntime`;
- `normalize_agent_args("opencode", [])` returns `["acp"]`;
- the preset catalog no longer contains an OpenCode duplicate;
- the built-in catalog entry exposes native model behavior and appropriate
  install/account-connection metadata;
- the settings harness row offers account connection from the catalog
  capability without checking the `opencode` runtime ID;
- ACP auth metadata for OpenCode produces the advertised terminal command
  without a Buzz-owned credential path;
- model discovery receives `opencode acp` and preserves provider-qualified
  model IDs; and
- affected desktop agent-config contract and onboarding acceptance tests still
  pass.

The full test suite for each touched package will run before completion. The
repository-level `just ci` gate will run before opening the pull request when
the local environment supports it.

## Non-Goals

- Adding OpenCode Go as a direct provider for the bundled Buzz Agent.
- Reimplementing OpenCode's model API or `/connect` flow.
- Storing OpenCode credentials in Buzz.
- Filtering or hard-coding OpenCode Go models.
- Changing unrelated harness catalog behavior or configuration UI.

## Success Criteria

- A user with the OpenCode CLI installed sees one OpenCode runtime in Buzz.
- Buzz can launch the official OpenCode login flow.
- After authentication, Buzz shows the model choices returned by OpenCode.
- A selected model is used when the managed agent starts through
  `opencode acp`.
- Users without OpenCode installed or authenticated receive actionable,
  non-secret-bearing errors.
