import path from "node:path";
import { fileURLToPath } from "node:url";
import { runFileSizeCheck } from "../../scripts/check-file-sizes-core.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

const MAX_LINES = 1000;

const rules = [
  { root: "src-tauri/src", extensions: new Set([".rs"]), maxLines: MAX_LINES },
  {
    root: "src/app",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/features",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/api",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/context",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/lib",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/ui",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: MAX_LINES,
  },
  {
    root: "src/shared/styles",
    extensions: new Set([".css"]),
    maxLines: MAX_LINES,
  },
];

// TEMP — these files exceed the 1000-line limit and are queued to be split.
// Do not add to this list; split the file instead. Remove each entry as its
// file is broken up. Tracked as a follow-up.
const overrides = new Map([
  // Native Builderlab auth/community commands add a small registration surface
  // to the existing Tauri composition root. The implementation lives in
  // builderlab.rs; this narrowly ratchets the command wiring while lib.rs is
  // queued for a broader composition-root split. Bumped for the
  // archive/unarchive/transfer community-management commands (web parity).
  ["src-tauri/src/lib.rs", 1013],
  // persona-events rebase: build_deploy_payload threads `state` for the
  // read-time relay-URL workspace fallback while keeping the create-time env
  // pin (the credential-leak guard). Load-bearing feature growth from the
  // rebase, queued to split with the rest of this list.
  // persona-refresh-on-spawn: re-snapshot + retain_managed_agent_pending call
  // in start_local_agent_with_preflight adds ~23 lines. Queued to split.
  // rebase onto main (2026-06-25): main's agents.rs grew by ~17 lines since
  // config-bridge: get_agent_config_surface/write_agent_config_field/put_agent_session_config
  // commands add ~40 lines. Queued to split.
  // branch cut; override bumped to cover the merged total. Queued to split.
  // persona-blank-fallback: persona_snapshot_with_agent_config_fallback call
  // sites add ~4 lines (extra fallback params + inline comments). build_deploy_payload
  // fix (blank-persona provider/model fallback) adds ~6 lines. Bug fix.
  // archive/mod_tests.rs carries the full test module for archive/mod.rs:
  // unit tests + 4 real-relay integration tests (ignored, live-relay only).
  // Production logic in mod.rs is now ~527 lines (under 1000). mod_tests.rs
  // is test-only content; the override covers the test growth accumulated
  // across the local-archive + agent-metric-archive PR series. store_tests.rs
  // (~731 lines) is under 1000 so needs no override.
  ["src-tauri/src/archive/mod_tests.rs", 1208],
  // unified-agent-model 1A.1: profile reconcile split to agents_profile.rs,
  // ratcheting 1443 -> 1295. Queued to split further in the A2 fold.
  // global-agent-config: resolve_deploy_model_provider + visibility exports
  // add ~40 lines on top of the 1A.1 ratchet. Queued to split.
  ["src-tauri/src/commands/agents.rs", 1340],
  // agent-lifecycle-fixes: cascade-delete in delete_persona restructured into
  // 3-phase (stage/stop/commit) + commit_cascade_agents injectable helper for
  // retry-safety. Load-bearing reviewer-required change; queued to split.
  // Consolidation removed the legacy persona-card import/export codecs.
  ["src-tauri/src/commands/personas/mod.rs", 984],
  // #1418 read-path fix: get_thread_replies' blocker fix (shared TIMELINE_KINDS
  // const + build_thread_replies_filter helper, mirroring the channel sibling so
  // the two p-gate filters can't drift) plus two guard unit tests. The file was
  // already at 995; this load-bearing correctness fix crossed 1000. Not generic
  // debt growth. Approved override; queued to split with the rest of this list.
  ["src-tauri/src/commands/messages.rs", 1082],
  // Residual repos_dir integration in ensure_nest_at: REPOS is provisioned
  // outside NEST_DIRS (it may be a symlink), so it needs its own create +
  // chmod-only-when-real-dir handling plus integration test coverage. The
  // self-contained repos_dir functions and their unit tests live in repos.rs;
  // this is the seam that must stay in nest.rs. Approved override; still queued
  // to split with the rest of this list.
  // dev-nest namespace: OnceLock<Option<PathBuf>> + init_nest_dir + constants
  // added to plumb the dev/prod discriminator. Load-bearing for the D2 nest fix.
  // dev-build CLI symlink: cli_link_name helper + is_dev param on
  // ensure_cli_symlink + prod/dev test variants add ~68 lines. Load-bearing;
  // queued to split with the rest of this list.
  // +4 lines: adopt shared create_symlink wrapper (behavior-preserving refactor
  // for multi-line rustfmt expansion of the skills symlink call site).
  // unified-agent-model 1A.1: inline test module moved to nest/tests.rs,
  // ratcheting 1575 -> 679 (under the 1000 default; entry kept as a ratchet).
  // observer-archive dev-default: path_is_dev_nest + nest_is_dev getters
  // (+25 lines) so observer_archive_default_enabled() keys off the dev nest.
  // Load-bearing; spends banked ratchet headroom, still well under 1000.
  ["src-tauri/src/managed_agents/nest.rs", 704],
  // keyring-dev-isolation: agent key migration added copy_agent_keys_between_stores
  // and load_readonly support; file grew past 1000 default. Queued to split.
  // +7 for try_delete_agent_key result-returning seam (snapshot-import rollback).
  // +48 (1335 -> 1383): agents-everywhere pair re-key — pair-scoped runtime
  // receipts (write_agent_runtime_receipt atomic JSON + remove/read_all
  // helpers) replace the pubkey-keyed PID file, plus the hashed pair-scoped
  // runtime log path. Load-bearing crash-recovery surface; queued to split.
  ["src-tauri/src/managed_agents/storage.rs", 1383],
  // harness-persona-sync: persona-runtime resolution threaded into the spawn
  // path here. Load-bearing feature growth; queued to split in the resolver
  // unify refactor followup. +26 for resolve_effective_prompt_model_provider
  // re-introduced after 826d735fe removal (config-bridge caller still needs it).
  // PGID resolution helper + PID-recycling safety guard added for orphan sweep.
  // activity-feed threads avatar_url into build_managed_agent_summary for the
  // assistant-bubble pinned snapshot.
  // +1 for agent_pubkey field in setup payload (config-nudge card wire).
  // persona-blank-fallback: resolve_effective_prompt_model_provider gains a
  // record_provider param + applies persona_field_with_record_fallback. +5 lines.
  // global-agent-config: spawn_agent_child loads global config and merges as
  // lowest env layer (+8 lines). Queued to split.
  ["src-tauri/src/managed_agents/runtime.rs", 2216],
  // config-bridge setup-payload env-boundary fix adds readiness wiring in
  // spawn_agent_child; load-bearing security fix, queued to split.
  ["src-tauri/src/managed_agents/config_bridge/reader.rs", 1016],
  // config-bridge-aware requirements: goose_requirements + injection tests
  // (4 new tests in goose_file_config_tests module) + test-determinism fixes
  // for the 3 existing goose tests that previously read real disk config.
  // New file in this PR; queued to split.
  // +2 readiness integration tests for flat-DATABRICKS_HOST canonicalization fix.
  // +1 cargo fmt whitespace reformat (readiness.rs closures inline after rebase).
  // +2 unit tests for cli_login_requirements resolve_command integration (DMG PATH fix).
  // Doctor-CTA: reworked cli_login_requirements to carry AcpAvailabilityStatus,
  // skip login probe for not-installed/adapter-missing/cli-missing states, and
  // added 4 unit tests covering each arm. Load-bearing discoverability fix.
  // Updated existing codex_not_ready test to use make_cli_runtime stub.
  // +4 lines: #1640 persona-env-vars-refresh rebase added availability-classification
  // growth in the live-persona env merge path. Feature plumbing, not generic debt.
  // Windows-CI portability: replaced POSIX true/false probes with current_exe()
  // stand-in + present_binary_str()/static_commands() helpers (+29 lines).
  // Tests now pass on windows-latest CI shard without POSIX shell utilities.
  // databricks-v1-to-v2-migration: databricks-v2 hyphen-alias added to all
  // host/credential match arms + 30+ readiness tests for provider aliases,
  // missing-host, and DATABRICKS_MODEL fallback. Load-bearing correctness fix.
  // #1613 augmented-PATH readiness probes grew the file +3 past the prior cap.
  // +16: resolve_effective_agent_env + global-config readiness wiring (#1448).
  // +1 rebase merge: GlobalAgentConfig import added alongside AcpAvailabilityStatus.
  // +2 rebase onto #1667: behavioral quad fields in AgentDefinition/ManagedAgentRecord.
  // +3 rebase onto main (#1568 + #1613): identity-import-keyring + augmented-PATH probes.
  // +18: CliConfigInvalid requirement surface for config-parse probe classification —
  // new Requirement variant + updated cli_login_requirements + 3 new probe-layer tests.
  // Load-bearing UX fix (bad config → clear diagnostic, not "run codex login").
  // codex-acp-package-swap: AdapterOutdated version-probe in cli_login_requirements
  // (+22 lines). Load-bearing — blocks login gate for deprecated 0.16.x adapter.
  // code-reviewer fix-round: codex readiness gate tests — 2 new tests for
  // outdated-adapter and garbage-version-output paths through the codex id gate
  // (+140 lines: make_codex_runtime helper, PATH_MUTEX serializer, 2 test fns).
  // Load-bearing test coverage; queued to split with the file generally.
  // +1: pub(crate) mod cli_probe declaration for doctor auth probe access.
  // +3: auth_probe_args: None + login_hint: None added to make_cli_runtime and
  // make_codex_runtime stubs (new KnownAcpRuntime fields).
  // Git Bash readiness is intentionally colocated with buzz-agent's other
  // setup-mode requirements. The Windows-only requirement and serialization
  // test add eight lines; split remains queued with the existing file debt.
  // Windows Doctor install fix: cli_install_commands_windows field added to test stubs.
  // team-instructions-first-class: ManagedAgentRecord fixture gains the new
  // team_id field (+1 line).
  ["src-tauri/src/managed_agents/readiness.rs", 1765],
  // applyWorkspace reposDir parameter plus the validateReposDir binding,
  // threaded through Tauri invokes for configurable repos_dir, plus the
  // harness-persona-sync `harnessOverride` create-input bit — load-bearing
  // parameter plumbing, not generic debt growth. Approved override; still
  // queued to split. Read-path lanes 1+2 add server-side fetch bindings
  // (getThreadReplies + getChannelMessagesBefore) and paged people-search
  // reachability — load-bearing reachability plumbing, not generic debt.
  // #1418 read-path fix: +3 doc-only lines correcting the getThreadReplies
  // contract (replies-only, root excluded — the query keys on root_event_id,
  // which root rows lack). Documentation accuracy, not code growth.
  // linux-updater isAutoUpdateSupported() binding + onboarding has_profile_event field.
  // config-bridge-aware requirements: getRuntimeFileConfig command adds ~15 lines.
  // +26 lines from PRs landing on main between prior rebase and this rebase.
  // baked-env-required-badge: getBakedBuildEnvKeys wrapper adds ~16 lines. Queued to split.
  // restart-badge: started the queued split — start/stopManagedAgent moved to
  // tauriManagedAgents.ts; limit ratcheted down 1388 → 1380 to bank the headroom.
  // identity-import-keyring: identity wrappers (RawIdentity, getIdentity, getNsec,
  // importIdentity, persistCurrentIdentity) moved to tauriIdentity.ts;
  // limit ratcheted down 1380 → 1360 to bank the headroom (absorbs main-side
  // growth landed between the split and the rebase).
  // mention-alias fix: profile wrappers (RawProfile/RawUserProfileSummary types,
  // getProfile/updateProfile/getUserProfile/getUsersBatch/searchUsers) moved to
  // tauriProfiles.ts; limit ratcheted down 1360 → 1241 to bank the headroom.
  // baked-env fold-in: getBakedBuildEnv + BakedEnvEntry type adds ~28 lines.
  // doctor-npm-eacces-preflight: hint field on RawInstallStepResult + mapper
  // passthrough (+2 lines).
  // doctor-install-reliability: node_required + auth_status + login_hint fields
  // added to RawAcpRuntimeCatalogEntry + fromRawAcpRuntimeCatalogEntry mapper (+8).
  // codex-install-auto-restart: restarted_count + failed_restart_count added to
  // RawInstallRuntimeResult + fromRawInstallRuntimeResult mapper (+2).
  // Git Bash Doctor discovery adds the raw Tauri response and its camelCase
  // mapper. This is the existing API boundary; split remains queued.
  // team-instructions-first-class: createManagedAgent Tauri bridge threads the
  // new teamId input through to the backend (+1 line).
  ["src/shared/api/tauri.ts", 1305],
  // doctor-npm-eacces-preflight: hint field added to InstallStepResult (+1 line).
  // codex-acp-package-swap: "adapter_outdated" variant added to AcpAvailabilityStatus (+1 line).
  // doctor-install-reliability: AuthStatus tagged union + nodeRequired/authStatus/
  // loginHint fields on AcpRuntimeCatalogEntry (+14 lines). Load-bearing new feature.
  // agent-lifecycle-fixes: GlobalAgentConfigSaveResult type grows with
  // failed_restart_count (+2 lines). Queued to split with the rest of this list.
  // mcp-readonly-view rebase: PR2 MCP config surface FE-type fields force +1 over the grandfathered ceiling.
  // Git Bash prerequisite payload adds four fields to the shared Tauri API
  // contract. This is the canonical type location; split remains queued.
  // signout-wipe: resetFailed field added to Identity type (+6 lines).
  // team-instructions-first-class: CreateManagedAgentInput.teamId (+2, incl.
  // doc comment) and AgentTeam/CreateTeamInput/UpdateTeamInput.instructions
  // (+3) — the new team-id spawn link and the runtime-layered instructions
  // field.
  ["src/shared/api/types.ts", 1047],
  // readiness-gate: PersonaDialog.tsx threads computeLocalModeGate +
  // requiredCredentialEnvKeys + RequiredFieldLabel so the "New agent" dialog
  // shows required markers and credential amber rows (parity with
  // CreateAgentDialog). +23 lines of gate wiring. Queued to split.
  // config-bridge-aware requirements: useRuntimeFileConfigQuery wiring adds
  // ~16 lines. Queued to split.
  // baked-env-required-badge: useBakedBuildEnvKeysQuery + bakedEnvKeys wiring
  // + correct exclusion-semantics for requiredEnvKeys adds ~14 lines.
  // +2 lines: filter managed provider key from requiredEnvKeys (suppress dead-input locked row).
  // global-agent-config parity: wire useGlobalAgentConfig into PersonaDialog
  // (Gap A: global-aware computeLocalModeGate + drop bare requiredCredentialEnvKeys;
  // Gap B: hasAutoOpenedAdvancedRef auto-expand effect) + effective-provider
  // save gate + Inherit/Select-a-provider label. Queued to split.
  ["src/features/agents/ui/PersonaDialog.tsx", 1080],
  // harness-persona-sync feature growth, queued to split in the resolver-unify
  // refactor followup. discovery.rs is dominated by the new test module
  // (the effective_agent_command / divergent / create-time override matrix);
  // alias-preservation coverage extends that matrix so create-time persona
  // agents keep an installed runtime alias when the primary command is absent.
  // Load-bearing, not generic debt.
  // config-bridge: schema-driven field extraction adds ~26 lines. Queued to split.
  // config-parity: max_tokens_env_var + context_limit_env_var fields added to
  // KnownAcpRuntime (2 fields × 4 runtimes + discovery tests = ~13 lines).
  // Load-bearing — required for buzz-agent normalized config parity.
  // same-runtime-pin: update_time_agent_command_override + its override /
  // same-runtime / alias / sentinel / non-override / persona-less test matrix
  // (~135 lines, mostly tests) so a deliberate Custom pin survives the update
  // path instead of being dropped back to inherit. Load-bearing, not debt.
  // unified-agent-model 1A.1: inline test module moved to discovery/tests.rs,
  // ratcheting 1259 -> 802 (under the 1000 default; entry kept as a ratchet).
  // agent-config-propagation: the agent_command_override decision family
  // (divergent / create-time / update-time / apply) moved to
  // discovery/overrides.rs; ratcheting 802 -> 685 to bank the headroom.
  // codex-acp-package-swap: probe_codex_acp_major_version (+24 lines) +
  // AdapterOutdated version-gate in discover_acp_runtimes (+22 lines). Both
  // load-bearing — required to detect the deprecated 0.16.x adapter and
  // prevent silent relay breakage after the spawn-contract change.
  // codex-acp-package-swap follow-up: tempfile-based bounded stdout read
  // (+18 lines), codex_adapter_availability/is_outdated helpers (+16 lines),
  // cross-platform probe contract. All load-bearing — required for correct
  // probe behaviour on Windows and descendant-process edge cases.
  // doctor-install-reliability: refreshable login_shell_path cache,
  // find_nvm_default_bin + parse_semver_tag helpers, auth probe cache +
  // probe_auth_status/cached_auth_status, runtime_needs_npm, probe_args_for,
  // PartialEntry struct, and updated discover_acp_runtimes with parallel auth
  // probes. Load-bearing fresh-install reliability fixes. (+289 lines)
  // doctor-install-reliability review fixes: LoginShellPath enum + double-checked
  // locking, is_safe_nvm_tag security validation, classify_probe_output helper,
  // auth_probe_args on KnownAcpRuntime (removes probe_args_for indirection),
  // process-level timeout replacing inner-thread pattern. (+75 lines)
  // codex-install-auto-restart review-fixes: availability_drift pure predicate
  // + updated adapter_availability_cached() signature (Option return, cold=None)
  // prevents false restart badge on newly restarted agents. Correctness fix;
  // load-bearing — required by Thufir's IMPORTANT findings. (+15 lines)
  // Windows Doctor install fix: cli_install_commands_windows field, impl block
  // for cli_install_commands_for_os(), command_basenames() + .cmd/.bat resolution,
  // Windows well-known dirs in common_binary_paths(), login_shell_candidates(),
  // path_candidates_from_env_raw(). Load-bearing Windows platform support.
  // +13: fetch_login_shell_path_inner Windows guard (POSIX PATH → None).
  // resolve_git_bash made pub(crate) for Windows test access.
  // +1: login_shell_candidates doc comment expanded for resolve_bash_path.
  // Buzz-managed Node path helpers and resolution tests moved to
  // managed_node_paths.rs and discovery/tests/managed_path_resolution.rs;
  // ratcheting 1366 -> 1392 after adding the managed-path probes to discovery.
  ["src-tauri/src/managed_agents/discovery.rs", 1393],
  // rebase over codex-acp-package-swap: its version-probe tests union with the
  // doctor-install-reliability nvm/login-shell/semver tests — each side alone
  // stayed under the 1000 default; the union exceeds it.
  // Windows Doctor install fix: command_basenames, cli_install_commands_for_os,
  // and login_shell_candidates tests. Load-bearing platform-awareness coverage.
  // +132: pass 2 — five cfg(windows) behavioral tests: command_basenames .cmd/.bat
  // candidates, cli_install_commands_for_os PowerShell selection, login_shell_path
  // None regression, .cmd shim resolution, no-git-bash error hint.
  // +32: deterministic .cmd resolver + no-registry + install_shell_from tests.
  // Managed-path resolution test split to discovery/tests/managed_path_resolution.rs.
  ["src-tauri/src/managed_agents/discovery/tests.rs", 1273],
  // identity-import-keyring: the identity resolution state machine's behavioral
  // matrix (46 tests over FakeIdentityStore — probe × marker × file cells,
  // adoption / read-back-corruption / marker-failure arms, recovery-mode
  // gating). Load-bearing regression coverage for silent identity rotation,
  // not generic debt growth. Approved override; split if the matrix grows.
  ["src-tauri/src/app_state_tests.rs", 1420],
  // migration_tests.rs carries the harness-sync migration coverage plus the
  // patch_json_records owner-only writeback regression test (SECURITY.md:90
  // crash-safe 0o600 fallback). Load-bearing security + feature coverage, not
  // generic debt growth. Approved override; still queued to split. Event-sync
  // (persona/team event reconcile) tests were split out to event_sync_tests.rs
  // and the limit ratcheted 1410 → 1110.
  // unified-agent-model 1A.1: materialize tests live with their module in
  // migration/materialize.rs; ratchet held at 1110.
  ["src-tauri/src/migration_tests.rs", 1110],
  ["src-tauri/src/nostr_convert.rs", 1126],
  // degraded-network resilience: relay.rs grew past 1000 with the addition of
  // relay_error_message hint-capping (oversized-hint test via loopback TCP) and
  // the relay_admission freshness-verification test. The loopback mock was
  // hardened (std::net + request-read-before-write) adding ~10 lines.
  // Queued to split test helpers to relay/tests.rs.
  // +30 (1047 -> 1077): agents-everywhere pair re-key — query_relay_at_with_keys
  // (NIP-98 signed /query with explicit agent keys + optional x-auth-tag) for
  // bounded-auth agent relay-membership discovery. Load-bearing; queued to
  // split alongside the test-helper split.
  ["src-tauri/src/relay.rs", 1077],
  // degraded-network resilience: visibleChannelId field + getter/setter, NOTICE
  // handler for relay back-pressure, and rate-limit gate imports add ~74 lines
  // of load-bearing degraded-network recovery code. Queued to split.
  ["src/shared/api/relayClientSession.ts", 1096],
  // Boot-time event sync (persona/team/agent event reconcile) was split out
  // to event_sync.rs, ratcheting this limit 1575 → 1310. Remaining content is
  // the pre-identity data migrations; still queued to split further.
  // unified-agent-model 1A.1: materialize_agent_runtimes split to
  // migration/materialize.rs, ratcheting 1310 -> 1297.
  // databricks-v1-to-v2-migration: reconcile_databricks_v1_to_v2 migration
  // + inner fn with baked-env gate + 26 tests. Load-bearing correctness fix.
  // am review fix: also clear stale V1 model field on provider rewrite +
  // new model-clear test. Load-bearing chimera fix.
  // keyring-dev-isolation: run_boot_migrations wires agent-key migration.
  ["src-tauri/src/migration.rs", 1436],
  // onMarkRead + isUnread prop threading (mirrors the onMarkUnread prop
  // already here) for the single-toggle mark-read/unread menu item — a small
  // overage from load-bearing per-message plumbing, not generic debt growth.
  // Approved override; still queued to split with the rest of this list.
  ["src/features/messages/ui/MessageThreadPanel.tsx", 1006],
  // AgentConfigPanel footer fold into ProfileFieldGroup for the config-bridge
  // panel — a small overage from load-bearing UI plumbing, not generic debt
  // growth. Approved override; still queued to split with the rest of this list.
  // +135 for AgentInfoFocusedView/DiagnosticsFocusedView/ChannelsFocusedView
  // props restored after 826d735fe removal (UserProfilePanel.tsx still needs them).
  ["src/features/profile/ui/UserProfilePanelSections.tsx", 1140],
  // +14 for openEditAgent event subscription (config-nudge card "Open Edit Agent" action).
  // +11 for editAgentFocus state + initialFocus prop threading (deep-link granularity).
  ["src/features/profile/ui/UserProfilePanel.tsx", 1025],
  // PersistBackend enum + marker-on-keyring-success plumbing and its three
  // fail-closed regression tests (silent identity rotation on keyring outage).
  // A small overage from load-bearing security plumbing on a file already at
  // 893 lines, not generic debt growth. Approved override; still queued to split.
  // cross-process keychain race fix (D3): interprocess lock + BlobLockGuard +
  // uid-keyed lockfile path + behavioral tests add ~303 lines. Load-bearing
  // security fix for the lost-update race that stranded agent keys.
  // identity-import-keyring: KeyringLockedScreen, RecoveryScreen,
  // load_readonly + load_all_readonly + store_all for safe cross-service reads.
  // sign-out wipe: delete_all() method removes the entire keychain blob under
  // the interprocess advisory lock; +8 lines. Load-bearing; queued to split.
  // signout-wipe phase 2: delete_all_with_legacy_cleanup replaces delete_all;
  // reads blob keys + deletes per-key legacy entries to prevent resurrection.
  // + regression test for per-key resurrection via real OS keychain.
  // Net growth ~36+32 lines over prior cap. Load-bearing correctness fix.
  // signout-wipe pass-2 (F2): delete_all_with_legacy_cleanup DPK deletes now
  // observable (propagate real errors); verify_fully_wiped checks all three
  // keychain shapes (main blob, DPK blob, per-key "identity"). +73 lines.
  ["src-tauri/src/secret_store.rs", 1307],
  // sign-out wipe: Sign Out section (AlertDialog + controlled state) added
  // at the bottom of the Profile settings page. Load-bearing UX feature;
  // queued to split when ProfileSettingsCard is broken into sub-components.
  // +20 lines: scroll-position save/restore across avatar editor open/close
  // to prevent layout shift from the Sign Out section causing a viewport jump.
  // +11 lines: signout-dev-webview-state — clear localStorage/sessionStorage
  // on successful signOut() resolve so dev-build webview state doesn't survive
  // a reset and vouch for the fresh key. Comment explains the race/redundancy.
  ["src/features/settings/ui/ProfileSettingsCard.tsx", 1044],
  // keyring-dev-isolation: keyring_service() fn (7 lines) replaces the const
  // to return "buzz-desktop-dev" in debug builds. Load-bearing isolation fix.
  // +10 (1042 -> 1052): media_fetch_client with redirect::Policy::none() so a
  // relay 3xx cannot forward the minted auth header cross-origin (SSRF fix).
  // +16 (1052 -> 1068): extracted that client into `build_media_fetch_client()`
  // -> Result so the fail-closed invariant is testable (no silent redirect-
  // following fallback; startup panics loudly instead). The function belongs
  // here beside `build_app_state` and its sibling client; its doc comment
  // carries the load-bearing SSRF rationale. Extraction would only relocate,
  // not reduce, the security-critical code.
  // +5 (1068 -> 1073): merge with main, which independently added the
  // managed_agent_profile_reconcile_enabled flag (field + doc + init) under
  // its own 1042-line override. Union of two separately approved additions.
  // +8 (1073 -> 1081): agents-everywhere pair re-key — managed_agent_processes
  // and session_config_cache re-keyed by ManagedAgentRuntimeKey, the runtime
  // transition lock doc broadened to cover all protected-PID transitions, and
  // clear_agent_session_caches (per-pubkey retain) added alongside the
  // per-key clear. Load-bearing identity-contract change; queued to split.
  ["src-tauri/src/app_state.rs", 1081],
  // multi-slot splitting + no-op suppression (#1309): the ReadStateManager
  // class grew from ~700 lines to ~1019 with the addition of
  // splitContextsIntoBudgetedSlots (pure fn + 5 tests), publishSplitSlots,
  // publishOneSlot, deleteExtraSlots, and the no-op suppression integration
  // test. Load-bearing feature growth, queued to split publishSplitSlots path
  // into readStateManagerSplit.ts.
  ["src/features/channels/readState/readStateManager.ts", 1030],
  // review feedback on #1492 restored the two-line load-bearing comment
  // documenting why `lastMessageAt` must not be an `activeReadAt` fallback
  // (reply-inclusive; would clear unread state early). The file was already
  // at the 1000 ceiling; comment-only overage, not code growth. Queued to
  // split with the rest of this list.
  // member-agent-flags: messageProfiles merge + ref stabilisation split out to
  // useMessageProfiles.ts, ratcheting 1002 -> 972 (under the 1000 default;
  // entry kept as a ratchet). +7 rebase onto main (#1698 timeline-window
  // growth), 972 -> 979.
  ["src/features/channels/ui/ChannelScreen.tsx", 979],
  // forced-unread persistence: markChannelUnread now writes through to
  // forcedUnreadStore (localStorage) so the sidebar badge survives reload and
  // the rail observer can read it. Three clear points added (markChannelRead,
  // markAllChannelsRead, drainSyncedAdvances). Load-bearing fix, not generic
  // debt growth. Queued to split with the rest of this list.
  ["src/features/channels/useUnreadChannels.ts", 1022],
  // Shared UI was added to this guard after splitting globals/markdown so
  // large shared renderers cannot grow further while follow-up splits land.
  // +33 for config-nudge detect-and-render + author-auth gate (normalizePubkey guard).
  ["src/shared/ui/markdown.tsx", 2152],
  // +15 (2199 -> 2214): the video right-click Download/Copy menu's props,
  // hook wiring, and render slot. The stateful menu logic (~52 lines) was
  // extracted to useVideoContextMenu.tsx; what remains here is the component's
  // public interface (downloadUrl/filename props) and cannot move out.
  ["src/shared/ui/VideoPlayer.tsx", 2214],
  ["src/shared/ui/sidebar.tsx", 1042],
  // permission-outcome (fix #1381 regression): pendingPermissions state map,
  // describePermissionOutcome helper, jsonRpcId key helper (handles both
  // string and finite-number JSON-RPC ids per spec), and the acp_write
  // response correlation branch are all tightly coupled to the existing
  // request handler. Load-bearing logic growth, not generic debt. Queued to
  // split into a dedicated permission module in the next transcript refactor.
  // +123: observer parity — 4 new named session/update classifier cases
  // (current_mode_update, usage_update, available_commands_update,
  // config_option_update) + replaceLifecycleItem helper for usage coalescing +
  // system-prompt ordering fix (turnId: null for per-channel items).
  // +35: session/new reposition-on-refire fix — removeItem helper +
  // upsertMetadata restart branch (remove+sealOpenMessages+push instead of
  // replaceItem in-place) so system-prompt anchor moves to stream tail.
  // Load-bearing feature growth; queued to split in next transcript refactor.
  ["src/features/agents/ui/agentSessionTranscript.ts", 1202],
  // catalog module; agent_models.rs retains the thin wrapper (~50 lines).
  // File still exceeds 1000 due to OpenAI/Anthropic discovery + subprocess
  // fallback. Queued to split into dedicated discovery modules.
  // Kept activity-feed design fixture: realistic prompt context and tool-heavy
  // chatter for render-class test/reference coverage. Queued to split with the
  // rest of this list if it grows further.
  // +2: baked build env folded under merged_env in both get_agent_models and
  // discover_agent_models so in-process discovery sees baked provider config on
  // a GUI-launched DMG (the discovery_env_with_baked_floor fold).
  // +3: provider tri-state applied in update_managed_agent handler
  // (if let Some(provider_update) = input.provider { record.provider = provider_update; }).
  // +8: harness_override thread-through in update_managed_agent so a deliberate
  // Custom pin routes to update_time_agent_command_override (comment + call).
  ["src-tauri/src/commands/agent_models.rs", 1079],
  // global-agent-config: get_agent_config_surface / write_agent_config_field /
  // put_agent_session_config commands + GlobalAgentConfig serde types. New file
  // in this PR; queued to split with the command module refactor.
  // +17: baked-env-global-unify: BUZZ_AGENT_THINKING_EFFORT added to
  // is_safe_to_reveal allowlist + baked_env_thinking_effort_is_unmasked test.
  // +1: doctor-install-reliability: login_hint: None added to goose_runtime test stub.
  // +1: doctor-install-reliability review fixes: auth_probe_args: None added to stub.
  // +11 (1021 -> 1032): agents-everywhere pair re-key — session-cache reads in
  // get_agent_config_surface derive the ManagedAgentRuntimeKey (relay-URL
  // fallback resolution) and put_agent_session_config gains a relay_url param.
  // Load-bearing identity plumbing; queued to split.
  // +18 (1032 -> 1050): review fix — put_agent_session_config reads the pair
  // relay from the harness-attached payload relayUrl (with effective-relay
  // fallback for older harnesses) instead of a required arg the frontend
  // wrapper never passed, which silently broke the session-config cache.
  ["src-tauri/src/commands/agent_config.rs", 1050],
  // codex-install-auto-restart review-fixes: should_restart_after_install
  // takes pid_alive:bool (pure predicate, no OS-dependent call); 3 racy
  // cache tests replaced with 6 pure availability_drift predicate tests;
  // dead-pid non-happy-path added. All load-bearing correctness fixes.
  // (+17 lines net vs previous 1330 limit; rustfmt expanded some call sites)
  // Git Bash Doctor discovery exposes a narrow async Tauri command at the
  // existing discovery boundary. The ten-line addition preserves the platform
  // neutral frontend contract; split remains queued.
  // Windows Doctor install fix: resolve_install_shell() + install_shell_command()
  // returns Result (Windows Git Bash resolution, CREATE_NO_WINDOW, taskkill timeout
  // kill), cli_install_commands_for_os() callsite, unit tests for shell selection
  // and per-OS install command accessor. Load-bearing Windows platform support.
  // +53: pass 2 — three cfg(windows) install shell tests (resolve succeeds with
  // Git, error hint content, install_shell_command succeeds).
  // +8: install_shell_from pure seam extracted for deterministic testing.
  ["src-tauri/src/commands/agent_discovery.rs", 1523],
  // draft-persistence predicate: submit-time `loadDraft` check + inline comment
  // + deps-array entry in submitMessage closes the never-persisted-boundary
  // defect (Thufir Pass-3 finding). Load-bearing correctness fix; queued to
  // split MessageComposer into submit/edit/media sub-modules.
  // +18: pendingImetaForPersistRef (local snapshot ref) + synchronous restore
  // path writes in the draft-key effect body, fixing the image-drop bug on
  // top-level nav switch (StrictMode simulate-unmount race on remount).
  // +12 autoSubmitDraftKey/onAutoSubmitComplete props + onAutoSubmitCompleteRef
  // + mount-only useEffect for the Drafts-panel "Send message" confirm-dialog
  // flow. Load-bearing feature growth; queued to split with the rest of this
  // list.
  // +3: onLinkShortcutRef wiring (ref decl + editor option + assignment) for
  // the ⌘K link-editor shortcut, mirroring the existing onEditLinkRef
  // pattern. Queued to split with the rest of this list.
  // +35: persistent audience scope/hook wiring and chip component handoff. The
  // chip markup lives separately; remaining lines connect existing composer
  // send state to the audience store. Queued with the existing split.
  // +23: edit-to-add-mention notify (8ace8eed) — onEditSave/edit-branch
  // mentionPubkeys threading + two snapshot refs (extractMentionPubkeys,
  // ownerPubkey) feeding the newly-added-mentions diff. Diff logic itself
  // lives in threading.ts (diffAddedMentionPubkeys); this is the minimal
  // composer-side wiring. Queued to split with the rest of this list.
  ["src/features/messages/ui/MessageComposer.tsx", 1114],
  // global-agent-config: model-tuning section (BuzzAgentModelTuningFields via
  // EditAgentAdvancedFields) + providerValid gate + effectiveProvider derivation
  // + globalProvider threading into getPersonaProviderOptions. All load-bearing
  // feature logic; queued to split with the rest of this list.
  ["src/features/agents/ui/EditAgentDialog.tsx", 1088],
  // global-agent-config rebase over #1639: AgentInstanceEditDialog (renamed from
  // EditAgentDialog by #1639) gained initialFocus?/EditAgentFocusTarget prop
  // threading from the deep-link focus feature, and isEditAgentProviderSaveValid
  // extracted as a testable helper with originalRuntimeSupportsProvider to close
  // the runtime-switch hole in Will's (b) providerValid gate narrowing.
  // E2E-fix round: added globalProvider fallback to useRequiredCredentialState
  // call site and buzz-agent auto-expand effect for model-tuning knob visibility.
  // F1-fix: added globalEnvVars to useRequiredCredentialState so globally-satisfied
  // credential keys are excluded from requiredEnvKeyMissing (display/gate parity).
  // Feature logic, not generic debt. Approved override; still queued to split.
  // +23 rebase onto #1667: behavioral quad fields (respond_to/parallelism/toolsets)
  // plumbed through AgentInstanceEditDialog from PersonaAdvancedFields.
  // +2 provider-aware effort: model/provider props threaded to BuzzAgentModelTuningFields.
  // +15 provider/model dropdown fixes: useBakedBuildEnvKeysQuery + hideProviderIds
  // for Databricks v1 gate; prospectiveRuntimeId default fallback for builtins.
  // PR-B moves default/API-key derivation into shared hooks; the explicit
  // hidden-key projection keeps the top-level secret out of Advanced rows.
  ["src/features/agents/ui/AgentInstanceEditDialog.tsx", 1195],
  // AgentDefinitionDialog grew past 1000 with the following load-bearing fixes:
  // isRuntimeAutoSeededRef tracking for edit-mode seeding (Fizz shows models);
  // runtimeSupportsLlmProviderSelection guard on discovery provider (codex fix);
  // hideProviderIds computation for Databricks v1 gate. Queued to split.
  ["src/features/agents/ui/AgentDefinitionDialog.tsx", 1035],
]);

await runFileSizeCheck({
  projectRoot,
  rules,
  overrides,
  label: "Desktop",
  scriptPath: "desktop/scripts/check-file-sizes.mjs",
});
