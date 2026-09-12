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

## Continuation 0394e318 (still incomplete; do not publish)

Recovered 78f0c373 without restarting its implementation. This continuation adds:

- Shared `protocol.ts::serializeManagement` / `MANAGEMENT_WIRE_BYTES` for both encrypted transports. The wire ceiling remains exactly 32768 UTF-8 plaintext bytes; diagnostic envelope and NIP-59 consumers use that same constant.
- `host.ts` validates each **actual projected slot inventory** before changing any binding map or loaded revision, at startup and heartbeat adoption. The inventory includes retained assignment/configurations/history/actual run and repeated model/workspace fields. Adoption reserves 8192 bytes (the existing configuration budget) below the wire ceiling. Oversized saved catalogs remain public saved revisions, not loaded revisions; prior reports continue. Startup with an oversized retained catalog refuses and releases its own lock after successful teardown.
- Real long-workspace/200-character-model regression: a fitting catalog round-trips through both codecs, a 19-runtime catalog is saved but not adopted, prior inventory remains unchanged and publishable, and reopen refuses it. Exact-limit/multibyte-overflow tests exercise both producers.
- Synthetic authenticated service fixture: unauthenticated Stop does not invoke teardown; concurrent verified Stops share one failed teardown promise; subsequent Stop/status remain Unknown; reservations/host lock remain; duplicate Start refuses. No process/key deletion is authorized by failure.
- Broker assertion now reports mode plus underlying verification error, without changing its checks or 2000 ms deadline.

### Seven-point delivery status

1. **Registration/UI:** preserved prior implementation and evidence; unchanged here.
2. **Databricks v2:** NOT completed. No native helper, OS-token custody/refresh or model bridge was built. Bounded source check confirmed `crates/buzz-agent/src/auth.rs::PkceOAuthTokenSource::new_with_http_timeout` immediately derives/reads its file cache and `save` writes it. A Beehive-only custody seam at that owner is necessary; passing a fresh directory is not OS token storage. Do not run the existing helper as a substitute. No helper packaging recipe can be claimed until a working helper exists. Existing normal build owner is `cargo build --release -p buzz-agent` with pinned Rust 1.95.0; future bridge must use an isolated target output and include the built helper in export.
3. **Harness discovery:** still Buzz Agent PATH-only; Desktop-equivalent adapter/CLI/version rules remain required. Prior-art plan has exact source owners and current Pi correction. No discovery rewrite made.
4. **Models/effort:** existing OpenAI listing/custom model retained; Databricks models and supported harness-specific effort selection/launch propagation remain required. No new end-to-end Databricks/effort evidence.
5. **Relay:** URL/actual connection state retained; optional metadata name capture still missing. No network fetch added.
6. **Wire:** pre-adoption actual representation gate and shared producer/consumer ceiling added and tested. Residual: 8 KiB reserve is conservative headroom, NOT proof that arbitrarily growing assignment/run history fits forever. It also applies to legacy startup inventories; unusually large existing inventories can now refuse startup even if below 32 KiB. Assess this compatibility boundary before release. Existing send failure handling does not constitute a future-lifecycle fit proof.
7. **Lifecycle/failures:** new service teardown/unauthorized-stop coverage passes. Broader credential helper late-completion/cancellation/failure matrix remains incomplete. Existing active-run immutability fixture still passes.

### Validation and failure assessment

Artifacts: `/Users/loganj/.buzz/artifacts/beehive-host-settings-continuation-0394e318/`.

- Node 24.15.0 direct TypeScript `--noEmit`: passed (`typecheck-final.log`).
- Focused host settings: 6/6 (`settings-fixed.log`). Original test failure retained in `settings.log`: JSON round-trip omits an undefined `move` property; fixture now compares serialized representation, not an in-memory undefined key.
- Service: 3/3 (`service-failures.log`).
- Broker diagnostic run: 1/1, all existing modes (`broker-diagnostic.log`).
- One complete final touched-package suite: **159 passed, 2 failed, 19 skipped**, 180 tests, 32.063s (`full-suite.log`). NOT green. No full-suite rerun, deadline inflation, serialization or assertion weakening.
- Broker now identifies failing `ok` stimulus: `Conversation response timed out; check relay admission and local sign-in`. Actual delta inspection from 4bdb5f09 to 78f0c373 found no `broker.ts` diff; `acp.ts` adds resolvedProviderKey only under `if (plan.buzzProvider)`. This fixture has no buzzProvider and takes the legacy Databricks environment path. Thus this stimulus does not exercise the new provider-key launch branch. It still exercises shared supervisor/broker ownership; a targeted pass is not a root-cause diagnosis or release waiver.
- Additional final-suite failure: `conversation.test.ts:52`, first Start at line 75 received no management receipt within the existing polling bound. It uses the same synthetic legacy Databricks/conversation path, no buzzProvider. Causality is unestablished; no contention explanation is asserted and no flake-repair campaign was started.
- Reuse prior unchanged Bun renderer and synthetic PTY evidence; no new UI smoke claimed.

### Narrow review authority owners / remaining risk

Credential authority: `credential-store.ts`, `native-credentials.ts`, `credential-helper.ts`, `manager-credential.ts`, `settings-credentials.ts`, `manager-credential-child.ts`; native OAuth authority will be `crates/buzz-agent/src/auth.rs`, not Desktop cache access. Process authority: `host-service.ts` exact capability/nonce/HMAC, `host.ts` loaded revision/Stop fences, `owned.ts`, `broker.ts`, `acp.ts`. Wire authority: `protocol.ts`, `nostr-codec.ts`, `host.ts::applySettings` and slot `validateBindings`.

This is a bounded continuation handoff, not a genuine external/toolchain blocker and not completion of BUILD IT ALL. Native auth/discovery/effort/relay work remains unexhausted. No push/install/package publication, native build, real credential access, real relay/provider request, browser login, installed harness execution or existing host control. Both installed packages remain at the previously reported 4bdb5f09; they were not inspected or mutated here.
