# Host settings implementation checkpoint

This is an **incomplete candidate**, not the complete authorized prototype. No install or publication was performed. Runtime integration now includes in-run Databricks refresh, Desktop-backed detection and model-specific Buzz Agent effort. Final combined validation, relay integration and delivery remain required. Earlier chronological limitations below are superseded by the runtime integration section at the end.

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

1. **Databricks v2 setup, native PKCE, OS custody and model catalog implemented.** Explicit provider open still prefills the URL; browser sign-in now calls the shared Rust owner through a built macOS arm64 helper. Verified OS storage precedes public CAS; cancellation cannot publish. Headless catalog/launch-preflight refresh uses Beehive-only entries. See `native/README.md` for the export recipe and the remaining static-token lifetime limit of an already-running stock harness. No Desktop credentials or JS OAuth engine.
2. **Desktop-equivalent harness discovery/capability rules and effort are NOT implemented.** Only Buzz Agent PATH resolution is offered. No Codex version probe, adapter/CLI discovery contract, current Pi `buzz-pi-acp`, Goose/Claude/Codex provider combinations or model-specific effort propagation is claimed. Effort is explicitly unavailable, never guessed.
3. Optional relay display name is now captured by bounded NIP-11 metadata (relay integration dfa54c7b8). Exact URL and actual state remain authoritative; failure/stale/cancel yields URL-only.
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
2. **Databricks v2 continuation a7e21caa:** `PkceOAuthTokenSource::new_with_custody` bypasses all token-file reads/writes; the default Desktop/CLI cache behavior is unchanged. New `beehive-databricks` binary uses native Beehive OS entries, verified writes, exact-account cross-process locking, shared PKCE and the canonical v2 model union/filter implementation. Built release Mach-O arm64 and helper package at `/Users/loganj/.buzz/artifacts/beehive-databricks-a7e21caa/`; reproducible scoped recipe in `native/README.md`. Future exports must merge the helper into their Beehive `bin/`; no installed package was changed. Start obtains a refreshed access token, but a running stock harness does not yet have continuous OAuth refresh.
3. **Harness discovery:** still Buzz Agent PATH-only; Desktop-equivalent adapter/CLI/version rules remain required. Prior-art plan has exact source owners and current Pi correction. No discovery rewrite made.
4. **Models/effort:** existing OpenAI listing/custom model retained; Databricks now uses the shared native authenticated catalog and custom-model path. Supported harness-specific effort selection/launch propagation remains required. Synthetic Databricks native auth/store/refresh/catalog and host Save/Start/ACP evidence is recorded in the a7e21caa artifact; no live-provider or effort evidence is claimed.
5. **Relay:** integrated bounded optional NIP-11 name; exact URL/actual state retained. Fetch is unauthenticated, size/deadline bounded, cancellation/stale fenced; failures remain nameless.
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


## Databricks continuation evidence (a7e21caa)

- New helper built with pinned Rust 1.95.0, scoped `-p buzz-agent --bin beehive-databricks`, isolated artifact target. No native toolchain installation, product overwrite, push or install.
- Native package suite: 700 passed, 0 failed, 1 ignored (complete package, synthetic services, closed test environment); formatting checked. New custody test runs fake local discovery/token/browser callback, state rejection, verified store, failed store, refresh/reopen, actual v2 catalog union/embedding filter, and token-file exclusion. The production native custody write/read-back function is also tested with injected OS read/write closures for mismatch, missing, denial and workspace mismatch. Existing OAuth/cache tests remain green.
- Pinned Node 24.15.0 strict typecheck and 15 focused tests passed. Databricks Save/Start test reaches the actual host launch and ACP consumer with exact provider/host/model/bearer while prior active state remains unchanged, using an explicitly injected credential provider and synthetic ACP executable. Cancellation/late completion/public-secret exclusion covered. No combined Beehive rerun or reassessment of the two previously recorded suite failures.
- Installed smoke loader now explicitly blocks the new native path. No real environment Databricks host, tokens, Keychain entries, retained OAuth state, browser login, production provider or installed harness was inspected/executed.
- This supersedes only earlier Databricks-unimplemented statements. The larger owner request and delivery are not complete. The helper package must be incorporated into subsequent exports; continuous refresh inside an already-running stock harness remains unsupported.


## Runtime integration 43ed57d2

- `spawnAgent` is now the common actual ACP-probe/conversation-harness spawn
  boundary. OS-backed Databricks bindings use a run-owned Node wrapper/proxy;
  native Rust remains the OAuth/custody owner. Each request re-enters the exact
  workspace/reference token source, including expiry refresh, without restarting
  the host/agent. Only an isolated per-run capability reaches the model harness;
  the relay runtime no longer receives the Databricks bearer. Stop aborts requests
  and helpers inside the existing supervised process group. Upstream 401/403 is
  latched, no redirect/workspace fallback, no new auth authority or token files.
- Current-main Desktop Rust catalog/preset data is exported as checked public JSON,
  including Pi `buzz-pi-acp` + underlying `pi`, separate adapter/CLI availability,
  Codex strict >=1.10.0 metadata probe, bundled/managed/PATH/login-shell/common/nvm
  lookup. Probes are bounded, output-capped and owned; no auth/status probes.
  Unix-only Beehive discovery uses login-shell PATH rather than shell aliases or
  functions; no Desktop Tauri IPC or credential/config reading is implied.
- The Add runtime picker shows installed/missing/incompatible adapters explicitly.
  This catalog's supported OS-backed provider combinations remain **Buzz Agent ×
  OpenAI/Databricks v2**. Other detected adapters are visibly unavailable for this
  provider catalog, not falsely offered universal credentials or effort. Existing
  Goose/Claude/Codex native-auth CLI setup paths remain intact; Pi execution/provider
  integration is not newly implemented.
- Effort comes from the packaged canonical model-capability manifest and exact/
  boundary-aware family precedence. Unknown custom models and UC FQNs omit the
  picker (no invented fallback effort). Inherit omits the variable. Explicit
  supported effort persists in the immutable runtime/provider binding, is checked
  again at launch, and reaches `BUZZ_AGENT_THINKING_EFFORT` in the actual harness.
  Save/adoption affects only new authorized runs; active snapshots remain unchanged.
- Installed-smoke loader explicitly blocks the new discovery/auth-runtime seams.
  No real Keychain, browser, provider, relay, profile, installed harness or existing
  host lifecycle was used. Native source/binary unchanged; reuse a7e21caa evidence
  and packaged helper. Exports must now include **src JSON files**, not just TS.

Evidence: `/Users/loganj/.buzz/artifacts/beehive-runtime-integration-43ed57d2/`.
Final focused integration: **25/25** (`focused-final.log`), including actual wrapper
single-run expiry/refresh/revocation and relay-transport bearer exclusion.
Bun 1.4.2 renderer: 2/2. Node 24.15.0 typecheck passes. The first renderer invocation
used a filter instead of an explicit path (no tests ran); corrected invocation is
saved separately. Initial broker/conversation invocation used repository-root CWD,
while these fixtures require package CWD (`resolve('test/...')`); both reported
process exit from nonexistent fixture paths. Correct package-CWD run: 3/3, unchanged
assertions/deadlines. These invocation errors do **not** explain the historical
4eada whole-suite timeout/missing-receipt failures. No combined suite was rerun.

Both old failing stimuli now pass through the shared `spawnAgent` function, but
lack `buzzProvider` and therefore retain identical legacy executable/args/env and
never enter the new proxy. Shared launch/ownership remains touched; Larry must
assess the final combined suite after relay integration, not waive older failures.
No push/install/publication. Capability-specific non-Buzz provider integration and
live-account readiness are not claimed by this component.


## Adapter continuation 9b0f8958 (incomplete assignment)

Relay-name commit is integrated (dfa54c7b8). Codex × saved OpenAI now uses the
existing Codex launch schema: detected >=1.10.0 adapter and separate CLI, saved
exact custom model, OS-only immutable credential reference, host admission read,
OPENAI_API_KEY and JSON CODEX_CONFIG model override, separate HOME/CODEX_HOME,
ACP protocol 2 exact-model evidence, existing owned Stop. No generic effort is
offered (Desktop agentConfigCore owns it via model ID). No new native code or
proxy routes. Legacy explicit Codex key-file bindings remain separate; an OS
binding cannot fall back to a file.

This does NOT complete the requested non-Buzz matrix. Pi execution/provider
integration and non-Buzz Databricks integration are still absent. Claude remains
provider-locked in the cited Desktop catalog; Claude × OpenAI is not offered.
Do not publish this as BUILD IT ALL complete. Current source inspection did not
establish a Desktop-owned Pi/Codex/Claude Databricks launch adapter; implementing
such transport still requires the actual adapter protocol/config contract, not
merely sending BUZZ_AGENT_PROVIDER to an unrelated process.

Evidence for this continuation, final suite and remaining work:
`/Users/loganj/.buzz/artifacts/beehive-adapter-integration-9b0f8958/INTEGRATION_RESULT.md`.

## Pi runtime continuation (supersedes Pi-disabled statements above)

Pi now uses the Desktop `buzz-pi-acp` fork plus a separately resolved `pi` CLI
with saved OpenAI and Databricks providers. Desktop imposes no Pi semver floor:
execution requires fork identity, ACP protocol 1, and positive exact model and
thinking config-option evidence before prompts. Both probe and conversation broker
use `_meta.systemPrompt` and immutable spawn-time provider/model/effort settings.

A run-owned wrapper creates an isolated Pi directory. Its public `models.json`
contains `$BEEHIVE_PI_KEY`, never a credential. OpenAI uses native Pi Responses;
Databricks uses shared-manifest Responses/Anthropic/MLflow routing, preserving raw
model/FQN and workspace. Its adapter receives only a private loopback capability;
every provider request enters the existing native workspace-bound token source.
No Desktop credentials, persistent Pi auth files, native OAuth copy or plugin
framework. Unknown/custom models remain exact and do not gain guessed effort.
Supported Pi thinking levels are the intersection of model capabilities and Pi's
native vocabulary. No selection is explicitly `off`, not retained user settings.

Save/registration remain inert; existing runs keep their snapshots. Explicit
Start/Stop own wrapper, adapter, Pi descendants, proxy and temporary directory.
No native binary change; exports still require `src/*.ts`, `src/*.json` and the
existing `bin/beehive-databricks`. Tests use explicit synthetic module/process
seams (`pi-isolation-loader.mjs`), fake external ACP executables and loopback
provider servers, not installed harnesses or real OS stores. Targeted cases live
in `pi-host.test.ts`, `pi-conversation.test.ts`, and `pi.test.ts`. This slice does
not claim completion of Codex/Claude or erase prior combined-suite failures.
