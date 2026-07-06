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
  ["src-tauri/src/commands/agents.rs", 1437],
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
  ["src-tauri/src/managed_agents/nest.rs", 1501],
  // harness-persona-sync: persona-runtime resolution threaded into the spawn
  // path here. Load-bearing feature growth; queued to split in the resolver
  // unify refactor followup. +26 for resolve_effective_prompt_model_provider
  // re-introduced after 826d735fe removal (config-bridge caller still needs it).
  // PGID resolution helper + PID-recycling safety guard added for orphan sweep.
  // activity-feed threads avatar_url into build_managed_agent_summary for the
  // assistant-bubble pinned snapshot.
  ["src-tauri/src/managed_agents/runtime.rs", 2150],
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
  // linux-updater-manual: isAutoUpdateSupported() binding adds 7 lines.
  ["src/shared/api/tauri.ts", 1347],
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
  ["src-tauri/src/managed_agents/discovery.rs", 1124],
  // migration_tests.rs carries the harness-sync migration coverage plus the
  // patch_json_records owner-only writeback regression test (SECURITY.md:90
  // crash-safe 0o600 fallback). Load-bearing security + feature coverage, not
  // generic debt growth. Approved override; still queued to split.
  ["src-tauri/src/migration_tests.rs", 1410],
  ["src-tauri/src/nostr_convert.rs", 1126],
  ["src/shared/api/relayClientSession.ts", 1022],
  ["src-tauri/src/migration.rs", 1575],
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
  // PersistBackend enum + marker-on-keyring-success plumbing and its three
  // fail-closed regression tests (silent identity rotation on keyring outage).
  // A small overage from load-bearing security plumbing on a file already at
  // 893 lines, not generic debt growth. Approved override; still queued to split.
  // cross-process keychain race fix (D3): interprocess lock + BlobLockGuard +
  // uid-keyed lockfile path + behavioral tests add ~303 lines. Load-bearing
  // security fix for the lost-update race that stranded agent keys.
  ["src-tauri/src/secret_store.rs", 1043],
  ["src-tauri/src/app_state.rs", 1033],
  // multi-slot splitting + no-op suppression (#1309): the ReadStateManager
  // class grew from ~700 lines to ~1019 with the addition of
  // splitContextsIntoBudgetedSlots (pure fn + 5 tests), publishSplitSlots,
  // publishOneSlot, deleteExtraSlots, and the no-op suppression integration
  // test. Load-bearing feature growth, queued to split publishSplitSlots path
  // into readStateManagerSplit.ts.
  ["src/features/channels/readState/readStateManager.ts", 1030],
  // Shared UI was added to this guard after splitting globals/markdown so
  // large shared renderers cannot grow further while follow-up splits land.
  ["src/shared/ui/markdown.tsx", 2119],
  ["src/shared/ui/VideoPlayer.tsx", 2199],
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
  // Load-bearing feature growth; queued to split in next transcript refactor.
  ["src/features/agents/ui/agentSessionTranscript.ts", 1167],
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
  ["src-tauri/src/commands/agent_models.rs", 1071],
]);

await runFileSizeCheck({
  projectRoot,
  rules,
  overrides,
  label: "Desktop",
  scriptPath: "desktop/scripts/check-file-sizes.mjs",
});
