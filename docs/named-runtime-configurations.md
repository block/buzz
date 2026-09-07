# Named runtime configurations

A configuration chooses a host, harness, exact model/provider, optional workspace,
and references to already-provisioned local credentials. Several configurations
can target the same host. Identity, persona and community memory do not move or
change when the next-launch selection changes.

## Scope and persistence

`ManagedAgentRecord.runtime_configurations` contains owner → community → set.
The existing agent identity remains global; the sets and Desktop IDs do not.
Absent sets migrate to Default. No unscoped prototype configuration is silently
assigned to the currently open community. Management verifies the active scope
and signed ownership, replaces exactly one set under the store lock, and persists
once. Host-local writes preserve other hosts' entries and reject changing them.
New/changed entries receive a new revision; stale whole-record writes fail.

The effective resolver only consumes the native launch projection, never guesses
a scope from the record's creation relay. Normal Start captures selection for its
actual owner/community pair. Explicit Start takes an exact `{id, revision}`;
null means Default, not “read selection again later”. The immutable `PreparedLaunch`
is not serializable. The management IPC retains names of local credential
references for editing, but no resolved values. Lifecycle catalogs omit workspace,
credential references, environment and identity material entirely.

## Launch and actual state

Preparation checks signed owner, exact local key identity, host, runtime,
workspace, readiness and the selected runtime's catalog-declared MCP executable.
Codex/buzz-agent declare `buzz-dev-mcp`; Goose/Claude do not declare a separate
server. Named launches refuse a missing/nonexecutable declared tool and reassert
its resolved command after inherited env. Default retains the existing optional
MCP skip behavior; optional git credential helpers are not configuration tools.

App preparation also binds effective team instructions and channel/thread session
partitioning: both change the session's instruction/context inputs. Edits to these
apply on the next preparation, rather than changing an in-flight preflight. Other
app state (replay floor, process nonce, logging, shutdown/admission fences) remains
invocation/operational policy, not a snapshot of all app state.

Ordinary async mesh preflight runs outside the transition lock. Callers fence owner/community after await, then revalidate the plan under
ordinary admission before shared spawn. Pair registration validates before
reaping an existing child and shared spawn checks again. Stop timestamps alone
do not invalidate a plan; changes to its launch inputs do. Scope changes and
configuration revision changes cannot silently substitute another launch. Ordinary
Default selection is fenced too, without imposing strict readiness on its legacy
setup path. App-launch restore captures/preflights the scoped selection and checks
the current record before terminating untracked pair state.

The running pair and durable runtime receipt stamp the launched reference.
Selection and edits never rewrite that receipt. A different already-running
configuration returns Stop-before-Start rather than satisfying the new request.
The existing Stop/generation/authority fences still apply. Ordinary Default Start
keeps its legacy setup behavior; explicit/catalog Default checks readiness.

A native spawned child is not proof of a successful ACP session or selected model.
Strict model enforcement belongs to ACP's session boundary; no provider model
fallback is an acceptable success for a named selection. The final native env
sets `BUZZ_ACP_REQUIRED_MODEL` to the exact prepared wire ID after inherited,
harness and mesh writes; non-Claude `BUZZ_ACP_MODEL` is pinned too. Default removes
both strict variables. Claude retains A1 (`ANTHROPIC_MODEL`, no `BUZZ_ACP_MODEL`).
ACP requires matching fresh session/current-model or verified switch evidence
before any prompt, not merely catalog membership or a successful child spawn. Remote lifecycle policy
is independent: this feature does not remove the existing keyless broker gate,
transfer keys, or prove cross-host Move.

## Regression evidence

`runtime_configurations/tests.rs` binds migration, scope-preserving writes,
foreign refs/hosts, plan revalidation and shared child spawn. The child is a shell
fixture, not a real model. `RuntimeConfigurations.test.mjs` mounts the editor with
mock IPC and checks exact revision, next-launch-only writes, failure and unmount
fences. Neither test suite constitutes native/model or two-Desktop acceptance.
