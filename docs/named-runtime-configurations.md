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
before any prompt, not merely catalog membership or a successful child spawn.

## Remote Start credentials

Remote-initiated Start uses the same destination-local credential behavior as
ordinary local Start. The owner must independently provision the agent's matching
identity key on that host. Buzz does not transfer/distribute keys, issue a broker
session, or enroll a host through lifecycle messages. The native shared launcher
may pass that already-local key to its child just as ordinary Start does; lifecycle
requests/results never carry keys, resolved credentials, or local paths.

Catalog and preflight require the exact owner/community/agent/host configuration
ID and revision, matching local identity, executable harness/tools, workspace and
runtime/provider prerequisites. Launch reads bypass a warm keyring cache and never
migrate a missing key into existence. Missing, unreadable or wrong identity means
ineligible, not an eligible inventory host. Catalog eligibility is revalidated
after async preflight. It is advisory and expires, never launch authority.

Execution repeats those checks under the shared admission lock before destructive
Restart and after teardown, so a revoked key cannot be recovered from a captured
plan. Named launch success/failure saves only lifecycle metadata into a fresh raw
store; it never persists captured/hydrated credentials or migrates keys. Ownership
and generation reads likewise do not hydrate or migrate keys. The request deadline
also survives through Stop into shared spawn. Failure after Stop is Failed, not
Running or automatic resurrection. A missing key does
not hide a still-running process or block ordinary owner-authorized Stop. Move
preflights before source Stop and rechecks expiry and exact revision afterward;
same-host switches use these same fences. General host inventory is retained;
selection and running configuration are separate. No new provisioning UX is implied.

## Regression evidence

`runtime_configurations/tests.rs` binds migration, scope-preserving writes,
foreign refs/hosts, plan revalidation and shared child spawn. The child is a shell
fixture, not a real model. `RuntimeConfigurations.test.mjs` mounts the editor with
mock IPC and checks exact revision, next-launch-only writes, failure and unmount
fences. `runtime_configurations/tests/remote_credentials.rs` exercises the signed
receiver and shared launch with synthetic identities and bounded child processes,
including actual Stop-time credential/executable loss and post-Stop expiry. Run it
on Unix with `cargo test --manifest-path desktop/src-tauri/Cargo.toml
--no-default-features managed_agents::runtime_configurations::tests::remote_credentials
-- --test-threads=1` (seven tests; default system-keyring builds exclude the fixture).
These fixture suites do not constitute real-model or two-Desktop acceptance.
