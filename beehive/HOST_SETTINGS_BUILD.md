# Host settings implementation checkpoint

This is an **incomplete candidate**, not the complete authorized prototype. No install or publication was performed. The remaining Databricks and capability/discovery work is required before calling the owner request complete.

## Implemented

- Host settings has Agents/Providers/Runtimes on the left and five controls on the right: Register agent, Add provider, Add runtime, Start, Stop. Forms occupy the right pane. No Host JSON/details view or bottom Actions/Inspect menu. Remote Agents and CLI recovery remain supported; remote Restart remains refused. Tab/arrows/Enter/Esc and ordinary help/quit keys work without intercepting field input.
- First Register/Start on an unconfigured computer opens owner-public-key/relay setup. No controller-owner sign-in for local settings.
- Hidden nsec registration derives the public key on Node. Bounded read-only kind0 lookup verifies event signature/hash, author and kind, selects current metadata, sanitizes terminal content, and distinguishes no profile from unavailable relay. Registration stores and verifies the exact OS credential before public catalog persistence. Missing/mismatched retained keys refuse automatic repair. No assignment/journal is created by registration.
- OpenAI provider records use immutable Beehive OS entries, read-back verification and a public-only recovery-reference log before key writes. No Desktop credentials, API-key environment harvesting, keychain enumeration or plaintext fallback. A bounded Node helper owns native work and settles only after close.
- Independent append-only public catalogs live in `settings.json`, with short cross-process lock, atomic full-snapshot CAS, bounds and validation. Provider/model list lookup is bounded; a custom model remains available after listing failure. Buzz Agent on PATH is selectable.
- Saved runtime records are projected into existing authorized slots' real `harnessSetups` inventory. Remote Save selects the exact runtime fingerprint through the existing authority/CAS path. Next Start resolves its exact provider entry through the Node helper. No catalog save grants Start, invents a slot, changes conversation authority or starts anything. Slots without a service HOME/config context do not receive executable runtime projections.
- Existing host heartbeat adopts validated additive catalogs every two seconds, after validating all projections. An invalid replacement retains the prior effective catalog. Active run snapshots do not change. Saved and loaded revisions are separate in the UI/service status.
- Detached Node service control uses an owner-only directory, instance capability and HMAC-authenticated nonce-bound request/response over a private Unix socket. The capability is a local service-control secret, not an identity/provider credential. It is never transmitted. Start serializes reservations; duplicate Start requires a verified instance. There is no PID signaling/adoption. Unverified endpoints, legacy/stale locks and incomplete teardown remain Unknown. Stop awaits host teardown and control-endpoint closure. Quit leaves the service running. Relay status is taken from actual transport state, not inferred from saved config.

## Missing / limitations

1. **Databricks v2 PKCE and model catalog are NOT implemented.** The UI prefills the URL from Node's environment only on explicit form open, but reports browser sign-in unavailable and saves nothing. The shared Rust PKCE flow currently owns a file cache; an explicit bounded native bridge with Beehive OS token custody is still needed. Do not substitute Desktop caches or a JS OAuth engine.
2. **Desktop-equivalent harness discovery/capability rules and effort are NOT implemented.** Only Buzz Agent PATH resolution is offered. No Codex version probe, adapter/CLI discovery contract, current Pi `buzz-pi-acp`, Goose/Claude/Codex provider combinations or model-specific effort propagation is claimed. Effort is explicitly unavailable, never guessed.
3. Optional relay display name is not captured yet; URL-only footer is truthful. There is no fabricated relay name.
4. Catalogs are deliberately append-only. There is no edit/delete/migration UI. Credential-attempt references are retained for explicit recovery; a public save failure can leave an OS entry. Do not delete retained attempts automatically.
5. Local service currently requires a Unix socket path of at most 100 bytes and an owner-only host directory. Windows transport is not implemented. The intended desktop/laptop target is macOS.
6. Large catalogs still need an explicit management-wire headroom gate before adoption. The catalog permits up to 100 rows while a management message is bounded at 32 KiB; verify/restrict the projected inventory against that limit before release. The small integrated flow does not establish large-inventory behavior.
7. The complete provider/helper failure/cancellation and service teardown-failure regression matrix remains to be extended. Current tests cover identity read-back failure and missing retained key; inherited helper cancellation; verified service duplicate/reopen/Stop, failed startup, stale lock/unverified endpoint; and the real host catalog/Save/new-Start path with synthetic providers.

## Validation of this checkpoint

Pinned direct Node 24.15.0 and Bun 1.4.2. No workspace/root scripts or dependency/bootstrap changes.

- TypeScript `--noEmit`: passed.
- Targeted settings integration: 4 passed. Includes hidden-nsec decode and retained-key refusal, signed profile tamper rejection, OpenAI verified store/list/custom model, and actual host initial Start → save reusable provider/runtime → live inventory → Save next while active A remains unchanged → Stop → Start synthetic OpenAI runtime B. B's external ACP fixture verifies the exact model and injected synthetic key. Journal contains no provider key. Invalid catalog replacement retains loaded revision.
- Detached service/controller focused checks: passed after fixing fixture path length, observed endpoint-close race and obsolete Host-details expectations. Earlier failures remain in external logs.
- Bun renderer: 2 passed. Hidden/pasted input, normal field typing, right controls, disabled control, pending Esc, resize, multiline and quit ownership.
- Actual PTY settings workflow: first-use setup → OpenAI save → runtime list/model/save → nsec registration → quit. Uses committed `manager-installed-loader.ts`, an explicit synthetic credential file and fresh `beehive-pairing-cli-` HOME. Loader now replaces **new profile/provider/service seams too**; it never contacts a live relay/provider or starts a real service during UI smoke. Synthetic API key/nsec are absent from terminal output and public settings.
- One final whole-package run: **157 passed, 1 failed, 19 skipped**, 177 tests, 30.211s. Failure remains `test/broker.test.ts:31`, `assert.ok('value' in r)` without the underlying verification diagnostic. Baseline was 150 pass / 2 fail / 19 skip; the old helper cancellation/timeout test passed this run. No assertion relaxation, timeout inflation, full-suite retry or inferred contention diagnosis. Broker/acp launch code is touched by this candidate; this result must be assessed in review rather than automatically waived as unrelated.

External evidence: `/Users/loganj/.buzz/artifacts/beehive-host-settings-build-fb3f35bf/`.

## Publication/package boundary

Do not claim installed/delivered/live credentials verified. Only task files should be committed on `feat/beehive-opentui-57f9c8fa`; no push/PR/merge/default branch or process mutation. Continue missing functionality and narrow credential/process review before delivery. Export exact accepted commit including all new `beehive/src/*.ts`; use the existing package-local lock/runtime recipe. Each eventual occupied `opentui-current` is replaced with one current export, preserving settings/evidence/processes and no retained rollback copy. Installed synthetic smoke must use the extended loader, not fresh HOME alone.
