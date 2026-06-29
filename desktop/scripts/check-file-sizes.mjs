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
  ["src-tauri/src/commands/agents.rs", 1380],
  // Residual repos_dir integration in ensure_nest_at: REPOS is provisioned
  // outside NEST_DIRS (it may be a symlink), so it needs its own create +
  // chmod-only-when-real-dir handling plus integration test coverage. The
  // self-contained repos_dir functions and their unit tests live in repos.rs;
  // this is the seam that must stay in nest.rs. Approved override; still queued
  // to split with the rest of this list.
  ["src-tauri/src/managed_agents/nest.rs", 1450],
  // harness-persona-sync: persona-runtime resolution threaded into the spawn
  // path here. Load-bearing feature growth; queued to split in the resolver
  // unify refactor followup.
  ["src-tauri/src/managed_agents/runtime.rs", 2031],
  ["src-tauri/src/managed_agents/personas.rs", 1080],
  // Phase-2 inbound reconcile + review-fix cycle: reconcile_inbound_persona_event
  // dispatches 30175/30176/30177 inbound plus kind:5 tombstone consume
  // (reconcile_inbound_tombstone), the two apply_inbound_* fns, the
  // event_d_tag/parse_deletion_coordinate helpers, and the preserve/overwrite +
  // secret-injection + tombstone test coverage. Load-bearing feature growth,
  // queued to split with the list. The two `agents-data-changed` emits (live
  // UI refresh on inbound reconcile + tombstone) add the latest growth.
  ["src-tauri/src/commands/personas.rs", 1279],
  ["src-tauri/src/managed_agents/persona_card.rs", 1050],
  // applyWorkspace reposDir parameter plus the validateReposDir binding,
  // threaded through Tauri invokes for configurable repos_dir, plus the
  // harness-persona-sync `harnessOverride` create-input bit — load-bearing
  // parameter plumbing, not generic debt growth. Approved override; still
  // queued to split.
  ["src/shared/api/tauri.ts", 1209],
  // harness-persona-sync feature growth, queued to split in the resolver-unify
  // refactor followup. discovery.rs is dominated by the new test module
  // (the effective_agent_command / divergent / create-time override matrix);
  // alias-preservation coverage extends that matrix so create-time persona
  // agents keep an installed runtime alias when the primary command is absent.
  // types.rs adds the persona/instance harness fields. Load-bearing, not
  // generic debt.
  ["src-tauri/src/managed_agents/discovery.rs", 1085],
  ["src-tauri/src/managed_agents/types.rs", 1037],
  // migration_tests.rs carries the harness-sync migration coverage plus the
  // patch_json_records owner-only writeback regression test (SECURITY.md:90
  // crash-safe 0o600 fallback). Load-bearing security + feature coverage, not
  // generic debt growth. Approved override; still queued to split.
  ["src-tauri/src/migration_tests.rs", 1410],
  ["src-tauri/src/nostr_convert.rs", 1126],
  ["src/shared/api/relayClientSession.ts", 1022],
  ["src-tauri/src/migration.rs", 1449],
  // onMarkRead + isUnread prop threading (mirrors the onMarkUnread prop
  // already here) for the single-toggle mark-read/unread menu item — a small
  // overage from load-bearing per-message plumbing, not generic debt growth.
  // Approved override; still queued to split with the rest of this list.
  ["src/features/messages/ui/MessageThreadPanel.tsx", 1006],
  // useDueReminderBadgeCount hook call + sum to wire due-reminder count into
  // the Inbox nav badge — a small overage from load-bearing badge plumbing,
  // not generic debt growth. Approved override; still queued to split.
  ["src/app/AppShell.tsx", 1010],
  // PersistBackend enum + marker-on-keyring-success plumbing and its three
  // fail-closed regression tests (silent identity rotation on keyring outage).
  // A small overage from load-bearing security plumbing on a file already at
  // 893 lines, not generic debt growth. Approved override; still queued to split.
  ["src-tauri/src/app_state.rs", 1012],
  // multi-slot splitting + no-op suppression (#1309): the ReadStateManager
  // class grew from ~700 lines to ~1019 with the addition of
  // splitContextsIntoBudgetedSlots (pure fn + 5 tests), publishSplitSlots,
  // publishOneSlot, deleteExtraSlots, and the no-op suppression integration
  // test. Load-bearing feature growth, queued to split publishSplitSlots path
  // into readStateManagerSplit.ts.
  ["src/features/channels/readState/readStateManager.ts", 1030],
]);

await runFileSizeCheck({
  projectRoot,
  rules,
  overrides,
  label: "Desktop",
  scriptPath: "desktop/scripts/check-file-sizes.mjs",
});
