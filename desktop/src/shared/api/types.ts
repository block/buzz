export type ChannelType = "stream" | "forum" | "dm";
export type ChannelVisibility = "open" | "private";
export type ChannelRole = "owner" | "admin" | "member" | "guest" | "bot";

export type Channel = {
  id: string;
  name: string;
  channelType: ChannelType;
  visibility: ChannelVisibility;
  description: string;
  topic: string | null;
  purpose: string | null;
  memberCount: number;
  memberPubkeys: string[];
  lastMessageAt: string | null;
  archivedAt: string | null;
  participants: string[];
  participantPubkeys: string[];
  isMember: boolean;
  ttlSeconds: number | null;
  ttlDeadline: string | null;
};

export type ChannelDetail = Channel & {
  createdBy: string;
  createdAt: string;
  updatedAt: string;
  topicSetBy: string | null;
  topicSetAt: string | null;
  purposeSetBy: string | null;
  purposeSetAt: string | null;
  topicRequired: boolean;
  maxMembers: number | null;
  nip29GroupId: string | null;
};

export type ChannelMember = {
  pubkey: string;
  role: ChannelRole;
  isAgent: boolean;
  joinedAt: string;
  displayName: string | null;
};

export type CreateChannelInput = {
  name: string;
  channelType: Exclude<ChannelType, "dm">;
  visibility: ChannelVisibility;
  description?: string;
  ttlSeconds?: number;
};

export type UpdateChannelInput = {
  channelId: string;
  name?: string;
  description?: string;
  visibility?: ChannelVisibility;
  /** Omit to leave unchanged, `null` to clear (permanent), or a positive number of seconds to set. */
  ttlSeconds?: number | null;
};

export type SetChannelTopicInput = {
  channelId: string;
  topic: string;
};

export type SetChannelPurposeInput = {
  channelId: string;
  purpose: string;
};

export type CanvasResponse = {
  content: string | null;
  updatedAt: number | null;
  author: string | null;
};

export type SetCanvasInput = {
  channelId: string;
  content: string;
};

export type SetCanvasResult = {
  ok: boolean;
  eventId: string;
};
export type AddChannelMembersInput = {
  channelId: string;
  pubkeys: string[];
  role?: Exclude<ChannelRole, "owner">;
  expectedRelayUrl?: string;
  expectedSignerPubkey?: string;
};
export type AddChannelMembersResult = {
  added: string[];
  errors: Array<{
    pubkey: string;
    error: string;
  }>;
};

export type { Identity, IdentityStorage } from "./identityTypes";

export type Profile = {
  pubkey: string;
  displayName: string | null;
  avatarUrl: string | null;
  about: string | null;
  nip05Handle: string | null;
  ownerPubkey: string | null;
  /** True when a real kind:0 metadata event exists on the relay for this pubkey.
   * False for the synthesized fallback returned when no event is present.
   * Used by the onboarding gate to distinguish new users from returning users
   * whose display name happens to be empty. */
  hasProfileEvent: boolean;
};

export type UserProfileSummary = {
  displayName: string | null;
  /** Kind-0 `name` field, kept separate from `displayName` so @mention text
   * can be matched against either alias (agents/CLI resolve mentions against
   * `display_name` *or* `name` at send time). */
  name?: string | null;
  avatarUrl: string | null;
  nip05Handle: string | null;
  ownerPubkey: string | null;
  isAgent?: boolean;
};

export type UsersBatchResponse = {
  profiles: Record<string, UserProfileSummary>;
  missing: string[];
};

export type UserSearchResult = {
  pubkey: string;
  displayName: string | null;
  avatarUrl: string | null;
  nip05Handle: string | null;
  ownerPubkey: string | null;
  isAgent: boolean;
};

export type UserSearchPage = {
  users: UserSearchResult[];
  nextCursor: string | null;
};

export type UpdateProfileInput = {
  displayName?: string;
  avatarUrl?: string;
  about?: string;
  nip05Handle?: string;
};

export type PresenceStatus = "online" | "away" | "offline";

export type PresenceLookup = Record<string, PresenceStatus>;

export type UserStatus = {
  text: string;
  emoji: string;
  updatedAt: number;
};

export type UserStatusLookup = Record<string, UserStatus | null>;

export type {
  ProjectLocalRepository,
  ProjectLocalRepoSnapshot,
  ProjectRepoBranchResult,
  ProjectRepoCommit,
  ProjectRepoContributor,
  ProjectRepoCloneResult,
  ProjectRepoDiff,
  ProjectRepoDiffFile,
  ProjectRepoFile,
  ProjectRepoMergeResult,
  ProjectRepoPullResult,
  ProjectRepoPushResult,
  ProjectRepoSnapshot,
  ProjectRepoSyncStatus,
} from "./projectGitTypes";

export type RelayEvent = {
  id: string;
  /** Local-only render identity for optimistic events that are later acknowledged. */
  localKey?: string;
  pubkey: string;
  created_at: number;
  kind: number;
  tags: string[][];
  content: string;
  sig: string;
  pending?: boolean;
};

export type SendChannelMessageResult = {
  eventId: string;
  parentEventId: string | null;
  rootEventId: string | null;
  depth: number;
  createdAt: number;
};

export type FeedItemCategory =
  | "mention"
  | "needs_action"
  | "activity"
  | "agent_activity";

export type FeedItem = {
  id: string;
  kind: number;
  pubkey: string;
  content: string;
  createdAt: number;
  channelId: string | null;
  channelName: string;
  channelType?: string;
  tags: string[][];
  category: FeedItemCategory;
};

export type HomeFeed = {
  mentions: FeedItem[];
  needsAction: FeedItem[];
  activity: FeedItem[];
  agentActivity: FeedItem[];
};

export type HomeFeedMeta = {
  since: number;
  total: number;
  generatedAt: number;
};

export type HomeFeedResponse = {
  feed: HomeFeed;
  meta: HomeFeedMeta;
};

export type GetHomeFeedInput = {
  since?: number;
  limit?: number;
  types?: string;
};

export type {
  SearchHit,
  SearchMessagesInput,
  SearchMessagesResponse,
} from "./searchTypes";

// ── Relay Members ────────────────────────────────────────────────────────────

export type RelayMemberRole = "owner" | "admin" | "member";

export type RelayMember = {
  pubkey: string;
  role: RelayMemberRole;
  addedBy: string | null;
  createdAt: string;
};
export type RelayAgent = {
  pubkey: string;
  ownerPubkey: string | null;
  name: string;
  agentType: string;
  channels: string[];
  channelIds: string[];
  capabilities: string[];
  status: "online" | "away" | "offline";
  respondTo: RespondToMode | null;
  respondToAllowlist: string[];
};

export type ManagedAgentRuntimeLifecycle =
  | "starting"
  | "listening"
  | "waking"
  | "ready"
  | "failed"
  | "stopped";

export type ManagedAgentRuntimeStatus = {
  pubkey: string;
  /** Exact submitted descriptor, present only on startup reconcile results. */
  requestedRelayUrl?: string;
  /** Canonical, backend-owned pair identity component. Do not normalize in TS. */
  relayUrl: string;
  localSetup: boolean;
  lifecycle: ManagedAgentRuntimeLifecycle;
  pid: number | null;
  error: string | null;
  logPath: string | null;
};

export type ManagedAgentBackend =
  | { type: "local" }
  | { type: "provider"; id: string; config: Record<string, unknown> };

import type { RestartDiffEntry } from "./restartDiff";
export type { JsonValue, RestartChange, RestartDiffEntry } from "./restartDiff";
export type ManagedAgent = {
  pubkey: string;
  name: string;
  personaId: string | null;
  /**
   * The record's harness/runtime id (e.g. "goose", "my-custom-harness").
   * `null` means the agent inherits its harness from the linked persona.
   * Used to count agents referencing a harness definition (delete confirm).
   */
  runtime: string | null;
  teamId?: string | null;
  relayUrl: string;
  acpCommand: string;
  /** Resolved/effective harness command (persona-wins, override-honored). */
  agentCommand: string;
  /**
   * Explicit per-instance harness pin. `null` means the agent inherits its
   * harness from the linked persona's runtime. Lets the Edit dialog show
   * "Inherit from persona" vs a concrete pin.
   */
  agentCommandOverride: string | null;
  agentArgs: string[];
  mcpCommand: string;
  turnTimeoutSeconds: number;
  idleTimeoutSeconds: number | null;
  maxTurnDurationSeconds: number | null;
  parallelism: number;
  systemPrompt: string | null;
  avatarUrl: string | null;
  model: string | null;
  modelSource: "definition" | "global" | "instance_legacy" | null;
  /** LLM inference provider, from the agent's pinned record snapshot. */
  provider: string | null;
  /** True when the linked persona has been edited since this agent was created. */
  personaOutOfDate: boolean;
  /** True when this agent's linked persona no longer exists. */
  personaOrphaned: boolean;
  /**
   * `true` when the running process was spawned with a config that no longer
   * matches what a spawn would use today — a plain restart would change what
   * runs. Complements `personaOutOfDate` ("a respawn would change it").
   * Always `false` for stopped agents.
   */
  needsRestart: boolean;
  /** Non-empty iff `needsRestart` is true. Empty when Rust omits the field. */
  restartDiff: RestartDiffEntry[];
  /** Per-agent env vars. Layered on top of persona envVars. */
  envVars: Record<string, string>;
  status: "running" | "stopped" | "deployed" | "not_deployed";
  pid: number | null;
  createdAt: string;
  updatedAt: string;
  lastStartedAt: string | null;
  lastStoppedAt: string | null;
  lastExitCode: number | null;
  lastError: string | null;
  lastErrorCode: number | null;
  logPath: string;
  startOnAppLaunch: boolean;
  autoRestartOnConfigChange: boolean;
  backend: ManagedAgentBackend;
  backendAgentId: string | null;
  /** Who the agent should respond to. Maps to `buzz-acp --respond-to`. */
  respondTo: RespondToMode;
  /**
   * Normalized 64-char lowercase hex pubkeys. Used only when `respondTo` is
   * `"allowlist"`. Preserved across mode toggles.
   */
  respondToAllowlist: string[];
};

/** Inbound author gate mode. Mirrors buzz-acp's --respond-to CLI flag. */
export type RespondToMode = "owner-only" | "allowlist" | "anyone";

export type BackendProviderCandidate = {
  id: string;
  binaryPath: string;
};

export type BackendProviderProbeResult = {
  ok: boolean;
  name?: string;
  version?: string;
  description?: string;
  config_schema?: Record<string, unknown>;
};

export type RelayMeshConfig = {
  modelRef: string;
};

export type CreateManagedAgentInput = {
  name: string;
  personaId?: string;
  /** Team this instance was deployed from; controls runtime team instructions. */
  teamId?: string;
  relayUrl?: string;
  acpCommand?: string;
  agentCommand?: string;
  /**
   * True when `agentCommand` is a runtime command the caller deliberately wants
   * to preserve instead of inheriting the linked persona command. This covers
   * deploy-dialog runtime selections and discovered or installed aliases for the
   * same persona runtime id, while still ignoring missing-runtime fallbacks.
   */
  harnessOverride?: boolean;
  agentArgs?: string[];
  mcpCommand?: string;
  turnTimeoutSeconds?: number;
  idleTimeoutSeconds?: number;
  maxTurnDurationSeconds?: number;
  parallelism?: number;
  systemPrompt?: string;
  avatarUrl?: string;
  model?: string;
  provider?: string;
  envVars?: Record<string, string>;
  spawnAfterCreate?: boolean;
  startOnAppLaunch?: boolean;
  backend?: ManagedAgentBackend;
  /** Omitted uses the linked persona default, then `"owner-only"`. */
  respondTo?: RespondToMode;
  /**
   * Hex pubkeys to allow when `respondTo === "allowlist"`. Validated &
   * normalized server-side (must be 64 hex chars each).
   */
  respondToAllowlist?: string[];
  relayMesh?: RelayMeshConfig;
};

export type CreateManagedAgentResponse = {
  agent: ManagedAgent;
  privateKeyNsec: string;
  profileSyncError: string | null;
  spawnError: string | null;
};

export type ManagedAgentLog = {
  content: string;
  logPath: string;
};

export type CancelManagedAgentTurnResult = {
  status: "sent" | "no_active_turn";
};

/** Outcome of a live `switch_model` control frame; `failure` lands late. */
export type SwitchManagedAgentModelStatus =
  | "sent"
  | "turn_ending"
  | "switched"
  | "unsupported_model"
  | "no_active_turn"
  | "failure";

export type ControlResultFrame = {
  type: "cancel_turn" | "switch_model";
  status: string;
  modelId?: string;
  /** Opaque per-pick id echoed from the request; correlates late frames. */
  requestId?: string;
  /** Buzz channel UUID from the observer envelope; disambiguates channels. */
  channelId?: string | null;
};

export type GitBashPrerequisite = {
  available: boolean;
  path: string | null;
  installInstructionsUrl: string;
  installHint: string;
};

export type AcpAvailabilityStatus =
  | "available"
  | "adapter_missing"
  | "adapter_outdated"
  | "cli_missing"
  | "not_installed";

/** Authentication/login status for a CLI-based ACP runtime. */
export type AuthStatus =
  | { status: "logged_in" }
  | { status: "logged_out" }
  | { status: "config_invalid"; diagnostic: string }
  | { status: "not_applicable" }
  | { status: "unknown" };

export type AcpRuntimeCatalogEntry = {
  id: string;
  label: string;
  avatarUrl: string;
  availability: AcpAvailabilityStatus;
  command: string | null;
  binaryPath: string | null;
  defaultArgs: string[];
  mcpCommand: string | null;
  /** Environment variable used to apply the initial model, when supported. */
  modelEnvVar: string | null;
  /** Environment variable used to apply the selected LLM provider, when supported. */
  providerEnvVar: string | null;
  /** Environment variable used to apply thinking effort, when supported. */
  thinkingEnvVar: string | null;
  maxTokensEnvVar: string | null;
  contextLimitEnvVar: string | null;
  maxRoundsEnvVar: string | null;
  installHint: string;
  installInstructionsUrl: string;
  canAutoInstall: boolean;
  /** True when the runtime depends on a separately installed vendor CLI. */
  requiresExternalCli: boolean;
  underlyingCliPath: string | null;
  /** True when an npm adapter step is pending but Node.js / npm is absent. */
  nodeRequired: boolean;
  /** Login/auth status for CLI-based runtimes. */
  authStatus: AuthStatus;
  /** Hint for completing authentication; null when not applicable or already logged in. */
  loginHint: string | null;
  /** "builtin" (compiled in), "preset" (PATH-probed, not editable), or "custom" (user JSON). Controls UI editability. */
  source: "builtin" | "preset" | "custom";
  /**
   * Definition-level env vars for `source: custom` entries. Populated from
   * `HarnessDefinition.env` so saves don't erase existing vars. Absent for
   * builtin/preset entries.
   */
  definitionEnv?: Record<string, string>;
  /** Spawn-time parallelism cap; absent for uncapped harnesses. */
  maxParallelism?: number;
};

/** An AcpRuntimeCatalogEntry that is confirmed available — command and binaryPath are non-null. */
export type AcpRuntime = AcpRuntimeCatalogEntry & {
  availability: "available";
  command: string;
  binaryPath: string;
};

export type {
  InstallRuntimeResult,
  InstallStepResult,
} from "./installTypes";

export type AcpAuthMethod = {
  id: string;
  name: string;
  description: string | null;
  type: string | null;
  args: string[];
  command: string[];
  meta: unknown | null;
};

export type AcpAuthMethodsResult = {
  methods: AcpAuthMethod[];
};

export type ConnectAcpRuntimeResult = {
  launched: boolean;
};

export type CommandAvailability = {
  command: string;
  resolvedPath: string | null;
  available: boolean;
};

export type ManagedAgentPrereqs = {
  acp: CommandAvailability;
  mcp: CommandAvailability;
};

export type AgentModelsResponse = {
  agentName: string;
  agentVersion: string;
  models: AgentModelInfo[];
  agentDefaultModel: string | null;
  selectedModel: string | null;
  supportsSwitching: boolean;
};
export type AgentModelInfo = {
  id: string;
  name: string | null;
  description: string | null;
};

// ── Config bridge types ──────────────────────────────────────────────────────
export type ConfigOrigin =
  | "buzzExplicit"
  | "acpNativeRead"
  | "acpConfigOption"
  | "envVar"
  | "configFile"
  | "personaDefault"
  | "globalDefault"
  | "runtimeOverride"
  | "harnessConstraint"
  | "harnessDefault";

export type ConfigWriteMechanism =
  | { type: "respawnWithEnvVar"; envKey: string }
  | { type: "acpSetConfigOption"; configId: string }
  | { type: "acpSetSessionModel" }
  | { type: "gooseNativeConfigWrite"; configKey: string }
  | { type: "readOnly" };

export type NormalizedField = {
  value: string | null;
  origin: ConfigOrigin;
  writeVia: ConfigWriteMechanism;
  overriddenValue: string | null;
  overriddenOrigin: ConfigOrigin | null;
  /** True if this field must be set for the harness to function. */
  isRequired: boolean;
};

export type ConfigFieldType =
  | { type: "string" }
  | { type: "number" }
  | { type: "boolean" }
  | { type: "enum"; options: string[] };

export type ConfigField = {
  key: string;
  label: string;
  value: string | null;
  origin: ConfigOrigin;
  schemaType: ConfigFieldType;
  writeVia: ConfigWriteMechanism;
};

export type ConfigTierStatus = "available" | "pending" | "notApplicable";

export type ConfigSourceReport = {
  acpNative: ConfigTierStatus;
  acpConfigOptions: ConfigTierStatus;
  envVars: ConfigTierStatus;
  configFile: ConfigTierStatus;
  configFilePath: string | null;
  mcpConfigFilePath: string | null;
};

export type ExtensionEntry = { name: string; kind: string; enabled: boolean };

/** B5/I-7: a single adapter-advertised value for an ACP config option. */
export type AcpConfigOptionValue = { value: string; displayName?: string };

export type NormalizedConfig = {
  model: NormalizedField | null;
  provider: NormalizedField | null;
  mode: NormalizedField | null;
  thinkingEffort: NormalizedField | null;
  maxOutputTokens: NormalizedField | null;
  contextLimit: NormalizedField | null;
  systemPrompt: NormalizedField | null;
};

export type RuntimeConfigSurface = {
  runtimeId: string | null;
  runtimeLabel: string | null;
  isPreSpawn: boolean;
  normalized: NormalizedConfig;
  advanced: ConfigField[];
  extensions: ExtensionEntry[];
  sources: ConfigSourceReport;
  /** #3493: `true` when the surface was read from a user-set `CLAUDE_CONFIG_DIR` — drives the Keychain caveat note in the panel. */
  claudeConfigDirCustom?: boolean;
  /** B5: the adapter-advertised `thought_level` configId, discovered from the running session. Present only for claude after the first session. Drives the effort picker. */
  effortConfigId?: string;
  /** B5/I-7: adapter-advertised option values for the `thought_level` option — the picker renders these instead of hardcoded values. */
  effortOptions?: AcpConfigOptionValue[];
};

export type UpdateManagedAgentInput = {
  pubkey: string;
  name?: string;
  model?: string | null;
  provider?: string | null;
  systemPrompt?: string | null;
  /** Absent = don't touch. Present = replace the env_vars map entirely. */
  envVars?: Record<string, string>;
  parallelism?: number;
  turnTimeoutSeconds?: number;
  relayUrl?: string;
  acpCommand?: string;
  agentCommand?: string;
  /**
   * True when `agentCommand` is a runtime/Custom command the user deliberately
   * picked (the dialog is not inheriting). Preserves a pin that maps to the
   * linked persona's own runtime instead of letting the backend drop it back to
   * inherit. Ignored when `agentCommand` is absent or the inherit sentinel.
   */
  harnessOverride?: boolean;
  agentArgs?: string[];
  mcpCommand?: string;
  /** Absent = don't touch. Present = set the mode. */
  respondTo?: RespondToMode;
  /**
   * Absent = don't touch. Present = replace the allowlist with this list
   * (validated & normalized server-side).
   */
  respondToAllowlist?: string[];
};
// Persona (agent definition) types live in a sibling module to keep this
// file inside the repo-wide size ratchet; re-exported so import paths
// (`@/shared/api/types`) are unchanged.
export type {
  AgentPersona,
  CatalogSourceCoordinate,
  CreatePersonaInput,
  PersonaBehaviorInput,
  UpdatePersonaInput,
} from "./personaTypes";

// ── Team types ────────────────────────────────────────────────────────────────
export type {
  AgentTeam,
  CreateTeamInput,
  TeamCatalogSourceCoordinate,
  UpdateTeamInput,
} from "./teamTypes";

// ── Channel Template types ─────────────────────────────────────────────────────

export type TemplateBackend =
  | { type: "local" }
  | { type: "provider"; id: string };

export type TemplateAgentEntry = {
  personaId: string;
  runtime: string | null;
  model: string | null;
  role: string | null;
  backend: TemplateBackend | null;
};

export type TemplateTeamEntry = {
  teamId: string;
  runtime: string | null;
  model: string | null;
  backend: TemplateBackend | null;
};

export type ChannelTemplate = {
  id: string;
  name: string;
  description: string | null;
  channelType: "stream" | "forum";
  visibility: "open" | "private";
  canvasTemplate: string | null;
  agents: {
    personas: TemplateAgentEntry[];
    teams: TemplateTeamEntry[];
  };
  isBuiltin: boolean;
  createdAt: string;
  updatedAt: string;
};

export type CreateChannelTemplateInput = {
  name: string;
  description?: string;
  channelType?: string;
  visibility?: string;
  canvasTemplate?: string;
  agents?: {
    personas: TemplateAgentEntry[];
    teams: TemplateTeamEntry[];
  };
};

export type UpdateChannelTemplateInput = {
  id: string;
  name: string;
  description?: string;
  channelType?: string;
  visibility?: string;
  canvasTemplate?: string;
  agents?: {
    personas: TemplateAgentEntry[];
    teams: TemplateTeamEntry[];
  };
};

export type {
  ApprovalActionResponse,
  Workflow,
  WorkflowApproval,
  WorkflowApprovalStatus,
  WorkflowRun,
  WorkflowRunStatus,
  WorkflowSaveResult,
  WorkflowStatus,
  TraceEntry,
  TriggerWorkflowResponse,
} from "@/shared/api/workflowTypes";
export type {
  ContactEntry,
  ContactListResponse,
  PublishNoteResult,
  UserNote,
  UserNotesCursor,
  UserNotesResponse,
} from "./socialTypes";

export type ThreadSummary = {
  replyCount: number;
  descendantCount: number;
  lastReplyAt: number | null;
  participants: string[];
};

export type ForumPost = {
  eventId: string;
  pubkey: string;
  content: string;
  kind: number;
  createdAt: number;
  channelId: string;
  tags: string[][];
  threadSummary: ThreadSummary | null;
};

export type ForumPostsResponse = {
  posts: ForumPost[];
  nextCursor: number | null;
};

export type ThreadReply = {
  eventId: string;
  pubkey: string;
  content: string;
  kind: number;
  createdAt: number;
  channelId: string;
  tags: string[][];
  parentEventId: string | null;
  rootEventId: string | null;
  depth: number;
};

export type ForumThreadResponse = {
  post: ForumPost;
  replies: ThreadReply[];
  totalReplies: number;
  nextCursor: string | null;
};

/**
 * Forward keyset cursor for the server-side thread read (`get_thread_replies`).
 *
 * The event-id tiebreak is load-bearing: thread replies routinely share a
 * `createdAt` second (bursty threads), so a timestamp-only cursor would skip
 * every tied reply past the page limit. The pair `(createdAt, eventId)` orders
 * replies unambiguously and lets paging resume strictly after the last event.
 */
export type ThreadCursor = {
  createdAt: number;
  eventId: string;
};

export type ThreadRepliesResponse = {
  /** The reply subtree (chronological, oldest first), depth >= 1. Excludes the root event (relay keys on `root_event_id`, which a root row lacks); the caller already holds the root. */
  events: RelayEvent[];
  /** Present only when a full page was returned — pass back to fetch the next page. */
  nextCursor: ThreadCursor | null;
};

/**
 * Composite backward keyset cursor for channel-timeline paging via the bridge
 * (`getChannelMessagesBefore`).
 *
 * The event-id tiebreak is load-bearing for the dense-second case: the relay
 * orders `created_at DESC, id ASC` and advances past a second denser than one
 * page with `id > eventId`. A bare `createdAt` (`until`) cursor cannot escape
 * such a second — it re-returns the same slice forever, leaving older history
 * unreachable. `(createdAt, eventId)` moves strictly older every page.
 */
export type ChannelPageCursor = {
  createdAt: number;
  eventId: string;
};

export type ChannelMessagesPageResponse = {
  /** One keyset page of top-level history, relay order (newest first). */
  events: RelayEvent[];
  /** Present only when a full page was returned — pass back to fetch the next (older) page. */
  nextCursor: ChannelPageCursor | null;
};

// ── Global agent configuration ────────────────────────────────────────────────

/**
 * Global agent configuration defaults applied to ALL agents.
 *
 * Lowest user-settable layer — per-agent and persona values win on any key
 * collision. Mirrors the Rust `GlobalAgentConfig` struct.
 *
 * Precedence: baked floor < global < persona < per-agent.
 */
export type GlobalAgentConfig = {
  /** Global env vars injected into all agents unconditionally. */
  env_vars: Record<string, string>;
  /** Global fallback provider (e.g. "anthropic", "databricks_v2"). Null = no global default. */
  provider: string | null;
  /** Global fallback model identifier. Null = no global default. */
  model: string | null;
  /** Preferred ACP runtime for agents without a persona-specific runtime. */
  preferred_runtime: string | null;
};

/**
 * Result returned by `set_global_agent_config`.
 *
 * Mirrors the Rust `GlobalAgentConfigSaveResult` struct.
 */
export type GlobalAgentConfigSaveResult = {
  /** The persisted global config (after strip-on-write). */
  config: GlobalAgentConfig;
  /** Number of local agents successfully stopped and restarted. */
  restarted_count: number;
  /** Number of agents whose stop succeeded but respawn failed. */
  failed_restart_count: number;
};
