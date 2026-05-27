import { promises as fs } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");

const rules = [
  {
    root: "src-tauri/src",
    extensions: new Set([".rs"]),
    maxLines: 500,
  },
  {
    root: "src/app",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: 500,
  },
  {
    root: "src/features",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: 500,
  },
  {
    root: "src/shared/api",
    extensions: new Set([".ts", ".tsx"]),
    maxLines: 500,
  },
];

// Exceptions should stay rare and temporary. Prefer splitting files instead.
const overrides = new Map([
  ["src-tauri/src/managed_agents/nest.rs", 1420], // version-gated AGENTS.md + SKILL.md refresh + .agents/.claude symlink migration + ensure_skill_symlinks (all known providers) + managed section upsert + dynamic agent context + tests
  ["src-tauri/src/managed_agents/personas.rs", 950], // built-in persona system prompts (Solo + Kit + Scout) + merge_personas inequality checks + persona pack import/uninstall/list + uninstall safety check
  ["src-tauri/src/managed_agents/teams.rs", 580], // built-in team registry (Kit & Scout) + merge_teams + validate_team_deletion + JSON export/import + tests
  ["src-tauri/src/managed_agents/persona_card.rs", 970], // PNG/ZIP/MD persona card codec + pack-zip detection + nested root finder + provider/model/namePool fields + 27 unit tests
  ["src/app/AppShell.tsx", 835], // message edit state + handlers + ChannelPane edit prop threading + scrollback pagination + workflows view + projects view + memory-leak safeguards + home-badge state lifted here so it consumes the same NIP-RS read-state as the sidebar (single ReadStateManager) + dock bounce wiring + mark-all-read context + channel notification callback + desktopEnabled guard
  ["src/features/channels/hooks.ts", 550], // canvas query + mutation hooks + DM hide mutation
  ["src/features/channels/ui/ChannelManagementSheet.tsx", 800],
  ["src/features/channels/ui/ChannelPane.tsx", 525], // composer/timeline/sidebar orchestration + anchored agent activity footers + imetaMedia threading on editTarget
  ["src/features/channels/ui/ChannelScreen.tsx", 555], // profile panel state + mutual exclusion wiring + ProfilePanelProvider context + agent typing classification + imetaMedia projection on editTarget
  ["src/features/notifications/hooks.ts", 535], // notification settings + feed notification lifecycle + profile batch resolution + truncated-pubkey guard + badge state
  ["src/features/home/ui/HomeView.tsx", 505], // inbox/feed orchestration + thread context + reply/delete flow + NIP-RS read-state projection wiring (useHomeInboxReadState)
  ["src/features/messages/hooks.ts", 500], // message query/mutation hooks + optimistic updates
  ["src/features/messages/ui/MessageComposer.tsx", 800], // media upload handlers (paste, drop, dialog) + channelId reset effect + edit mode (pre-fill, save, cancel, escape) + composer autofocus (#572) + Sprout code-block paste branch (round-trips copy-button output as a literal codeBlock so Markdown can't reshape it) + scroll-to-bottom on multi-line paste (#619) + Slack-style attachment-editable edits: seed pendingImeta from edit target, stash/restore user's draft pendingImeta across edit-mode entry/exit, re-append imeta markdown lines on edit-submit so renderer draws them
  ["src/features/settings/ui/SettingsView.tsx", 600],
  ["src/features/sidebar/ui/AppSidebar.tsx", 860], // channels + forums creation forms + Pulse nav
  ["src/shared/api/relayClientSession.ts", 1040], // durable websocket session manager with reconnect/replay/recovery state + sendTypingIndicator + fetchChannelHistoryBefore + subscribeToChannelLive (huddle TTS) + subscribeToHuddleEvents (huddle indicator) + disconnect() for workspace switch teardown + fetchEvents/subscribeLive/publishEvent for NIP-RS read state + publishUserStatus/subscribeToUserStatusUpdates (NIP-38) + ConnectionState plumbing & stall-watchdog wiring for half-open WS detection (Warp orange-icon case) + terminal session latch (auth rejection no longer racing back to reconnecting) — emitter + watchdog + reconnect policy logic extracted to relayConnectionStateEmitter.ts / relayStallWatchdog.ts / relayReconnectPolicy.ts
  ["src-tauri/src/commands/media.rs", 730], // ffmpeg video transcode + poster frame extraction + run_ffmpeg_with_timeout (find_ffmpeg via resolve_command, is_video_file, transcode_to_mp4, extract_poster_frame, transcode_and_extract_poster) + spawn_blocking wrappers + tests
  ["src-tauri/src/commands/agents.rs", 881], // remote agent lifecycle routing (local + provider branches) + scope enforcement + persona pack metadata wiring + mcp_toolsets field + NIP-OA auth_tag in deploy payload
  ["src-tauri/src/commands/messages.rs", 515], // feed multi-query + NIP-50 search + forum thread resolution + thread ref + reactions via REQ + edit_message media_tags param (Slack-style attachment-editable edits)
  ["src-tauri/src/nostr_convert.rs", 1150], // 12 Nostr event→model converters (channels, profiles, members, notes, search, agents, relay members) + rank_user_search_results helper for NIP-50 user search + 33 unit tests
  ["src-tauri/src/managed_agents/runtime.rs", 1110], // ... + respond-to gate env (SPROUT_ACP_RESPOND_TO[_ALLOWLIST]) + per-mode env builder + tests + persona/agent env_vars spawn merge (helper + tests now in env_vars.rs)
  ["src-tauri/src/managed_agents/discovery.rs", 680], // KNOWN_ACP_PROVIDERS catalog + resolve_command cache + login_shell_path + classify_provider (four-state: Available/AdapterMissing/CliMissing/NotInstalled) + discover_acp_providers with dynamic install_hint + known_acp_provider/known_acp_provider_exact + normalize_agent_args + 15 unit tests
  ["src-tauri/src/managed_agents/types.rs", 745], // ManagedAgentRecord/Summary + Create/Update request structs + AcpProviderCatalogEntry + InstallRuntimeResult + RespondTo enum + validate_respond_to_allowlist + tests + persona/agent env_vars field
  ["src-tauri/src/managed_agents/backend.rs", 700], // provider IPC, validation, discovery, binary resolution + tests + redact_secrets_with for user env values + env_secrets_from_request + redact_env_values_in (shared with model discovery)
  ["src/features/huddle/HuddleContext.tsx", 650], // huddle lifecycle context + joinHuddle + connectAndSetupMedia shared helper + activeSpeakers/isReconnecting state + PTT (reusable AudioContext) + TTS subscription + mic level analyser (10fps throttle) + agent pubkey refresh
  ["src/features/agents/hooks.ts", 550], // agent query/mutation surface + useAvailableAcpProviders (type-narrowing filter hook) + useInstallAcpRuntimeMutation + built-in persona library activation
  ["src/features/agents/ui/AgentsView.tsx", 880], // remote agent lifecycle controls + persona/team management + persona import-update dialog wiring + built-in catalog/library state orchestration
  ["src/features/agents/ui/UnifiedAgentsSection.tsx", 570], // unified persona-grouped agent view with collapsible groups, bulk actions, drag-drop import, empty/loading states
  ["src/features/agents/ui/ManagedAgentRow.tsx", 530], // EditAgentDialog integration + provider/local branching
  ["src/features/agents/ui/TeamDialog.tsx", 530], // team create/edit dialog with persona multi-select, import button, window drag detection, removal confirmation
  ["src/features/agents/ui/TeamImportUpdateDialog.tsx", 660], // team import diff preview with member matching/updating/adding/removing sections, LCS line counts, removal confirmation
  ["src/features/agents/ui/useTeamActions.ts", 510], // team CRUD + export + import + import-update orchestration with query invalidation
  ["src/features/agents/ui/PersonaDialog.tsx", 515], // persona create/edit form + env vars editor + drag-drop file import + runtime provider dropdown with availability warnings
  ["src/features/agents/ui/CreateAgentDialog.tsx", 685], // provider selector + config form + schema-typed config coercion + required field validation + locked scopes
  ["src/features/channels/ui/AddChannelBotDialog.tsx", 690], // provider mode: Run on selector, trust warning, probe effect, single-agent enforcement, provider warnings display + RespondTo field + reuse guardrail
  ["src/features/settings/ui/ChannelTemplatesSettingsCard.tsx", 850], // template CRUD card + TemplateFormDialog (persona/team chip selectors + provider assignments + canvas template) + TemplateTeamSelector + ProviderAssignments + ProviderRow
  ["src/shared/api/types.ts", 650], // ... + AcpProviderCatalogEntry + AcpProvider (narrowed subtype) + InstallRuntimeResult + RespondToMode + respondTo/respondToAllowlist on ManagedAgent/Create/Update inputs
  ["src-tauri/src/events.rs", 810], // event builders + build_huddle_guidelines (kind:48106) + post_event_raw transport helper + participant p-tag on join/leave + NIP-43 relay admin builders (add/remove/change-role) + check_relay_role + DM/presence/workflow command builders + NIP-IA identity-archive builders (9035/9036) + .allow_self_tagging() guards (nostr 0.44 strips self-`p` by default; self-archive/unarchive needs it preserved) + spec vector 1 layout test
  ["src-tauri/src/huddle/mod.rs", 1020], // huddle state machine + Tauri commands + sync protocol doc; state/relay/pipeline extracted + emit_huddle_state_changed wiring
  ["src-tauri/src/huddle/models.rs", 950], // model download manager for Parakeet TDT-CTC STT + Pocket TTS with streaming downloads + SHA-256 verification + Rust-native tar extraction + version manifest + atomic swap + hot-start signaling + MODEL_LICENSE.txt sidecar (fail-closed readiness) + idempotent legacy Moonshine dir cleanup + tts_readiness_requires_license_sidecar test + Mary (VCTK p333) reference voice attribution block
  ["src-tauri/src/huddle/stt.rs", 580], // STT pipeline + PTT edge-detection flush + PTT gating (is_speech AND ptt_active) + barge-in for VAD mode + rubato resampler + earshot VAD + sherpa-onnx transcription
  ["src-tauri/src/huddle/pocket.rs", 900], // Pocket TTS engine wrapper + prepare_pocket_prompt (capitalize/punctuate/pad short inputs + insert ". . " sacrificial cold-start prefix, mirror upstream pocket-tts prepare_text_prompt) + build_generation_extra (only overrides max_frames, and only for ≤4-word inputs, to bound runaway "monster breathing" generation without clipping multi-clause sentences) + trim_leading_cold_start (post-synth strip of sacrificial audio on short prompts, workaround for kyutai-labs/pocket-tts #91 first-phoneme drop) + 18 unit tests including regressions for the static-burst-on-multi-clause-sentence bug and the first-phoneme-drop trim hazards
  ["src-tauri/src/huddle/preprocessing.rs", 670], // TTS text preprocessing pipeline + unified split_sentences + int_to_words 0-999999 + URL trailing punctuation preservation + 23 unit tests
  ["src-tauri/src/huddle/relay_api.rs", 520], // audio relay recv task + per-peer frame counting for remote human TTS interrupt + NIP-98 channel member query
  ["src-tauri/src/huddle/tts.rs", 1380], // TTS pipeline + session warmup + cancel/shutdown handling + apply_fade_out (fade-out only — leading fade removed 2026-05-18 after onset-attenuation regression measured in examples/pocket_onset_probe.rs) + FIRST_APPEND_LEAD_IN_SAMPLES + build_sentence_append_plan (pure helper enforcing the lead-in fires exactly once per utterance, not per sentence — see lead_in_pad_fires_exactly_once_per_utterance regression test) + normalize_for_playback (per-sentence peak normalization to -3 dBFS ceiling with MAX_GAIN cap) + 30 unit tests (18 interrupt + 5 fade-out + 1 first-append-lead-in + 3 build-sentence-append-plan + 6 normalize)
  ["src-tauri/src/relay.rs", 510], // +4 lines for NIP-OA auth tag injection in profile sync (build_profile_event) + verification test
  ["src-tauri/src/commands/pairing.rs", 600], // NIP-AB pairing actor: 3 Tauri commands + background WS task + NIP-42 auth + NIP-43 probe + event parsing helpers
  ["src-tauri/src/lib.rs", 730], // +4 lines for PairingHandle managed state + 3 pairing command registrations + parse_message_deep_link helper extracted with 6 unit tests covering empty-param filter regression
  ["src/shared/api/tauri.ts", 1212], // pairing command wrappers + applyWorkspace + NIP-44 encrypt/decrypt wrappers + observer_url field + relay member API functions (list/get/add/remove/change-role) + prevent sleep + AcpProviderCatalogEntry raw types + fromRawAcpProviderCatalogEntry converter + installAcpRuntime
]);

async function walkFiles(directory) {
  const entries = await fs.readdir(directory, { withFileTypes: true });
  const files = await Promise.all(
    entries.map(async (entry) => {
      const fullPath = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        return walkFiles(fullPath);
      }

      return [fullPath];
    }),
  );

  return files.flat();
}

function findRule(relativePath) {
  return rules.find((rule) => {
    const normalizedRoot = `${rule.root}${path.sep}`;
    return relativePath.startsWith(normalizedRoot);
  });
}

function countLines(content) {
  if (content.length === 0) {
    return 0;
  }

  return content.split(/\r?\n/).length;
}

const candidateFiles = (
  await Promise.all(
    rules.map((rule) => walkFiles(path.join(projectRoot, rule.root))),
  )
).flat();

const violations = [];

for (const filePath of candidateFiles) {
  const relativePath = path.relative(projectRoot, filePath);
  const rule = findRule(relativePath);
  if (!rule) {
    continue;
  }

  const extension = path.extname(relativePath);
  if (!rule.extensions.has(extension)) {
    continue;
  }

  const limit = overrides.get(relativePath) ?? rule.maxLines;
  const content = await fs.readFile(filePath, "utf8");
  const lineCount = countLines(content);
  if (lineCount > limit) {
    violations.push({
      limit,
      lineCount,
      relativePath,
    });
  }
}

if (violations.length > 0) {
  console.error("Desktop file size check failed:");
  for (const violation of violations) {
    console.error(
      `- ${violation.relativePath}: ${violation.lineCount} lines (limit ${violation.limit})`,
    );
  }
  console.error(
    "Split the file or add a narrowly scoped exception in `desktop/scripts/check-file-sizes.mjs`.",
  );
  process.exit(1);
}
