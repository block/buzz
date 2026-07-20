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

export type OpenDmInput = {
  pubkeys: string[];
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
};

export type AddChannelMembersResult = {
  added: string[];
  errors: Array<{
    pubkey: string;
    error: string;
  }>;
};

export type Identity = {
  pubkey: string;
  displayName: string;
  /** True when the app booted in "identity lost" recovery mode — the OS
   *  keyring was empty despite a prior successful migration. The frontend
   *  should route to nsec re-import instead of normal onboarding.
   *  Mutually exclusive with `locked`. */
  lost?: boolean;
  /** True when the app booted with an ephemeral key because the OS keyring
   *  holding the real identity is UNREACHABLE (e.g. GNOME Keyring / KWallet
   *  locked). The real key still exists; no in-app recovery is possible —
   *  the user must unlock the keyring externally and relaunch.
   *  Mutually exclusive with `lost`. */
  locked?: boolean;
  /** True when the boot-time Phase 2 reset attempted a wipe but verification
   *  failed. Identity resolution was skipped; the sentinel is preserved so
   *  the next relaunch retries the wipe automatically. */
  resetFailed?: boolean;
};

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

export type SearchMessagesInput = {
  q: string;
  limit?: number;
  channelId?: string;
};

export type SearchHit = {
  eventId: string;
  content: string;
  kind: number;
  pubkey: string;
  channelId: string | null;
  channelName: string | null;
  createdAt: number;
  score: number;
  threadRootId?: string | null;
};

export type SearchMessagesResponse = {
  hits: SearchHit[];
  found: number;
};

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
  name: string;
  agentType: string;
  channels: string[];
  channelIds: string[];
  capabilities: string[];
  status: "online" | "away" | "offline";
  respondTo: RespondToMode | null;
  respondToAllowlist: string[];
};

export type ManagedAgentBackend =
  | { type: "local" }
  | { type: "provider"; id: string; config: Record<string, unknown> };

export type ManagedAgent = {
  pubkey: string;
  name: string;
  personaId: string | null;
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
  /** LLM inference provider, from the agent's pinned record snapshot. */
  provider: string | null;
  /**
   * `true` when the linked persona has been edited since this agent was
   * created — the running agent uses the older pinned snapshot. Surface a
   * "out of date" marker and prompt the user to delete + respawn to update.
   * Always `false` for non-persona agents and for orphaned agents.
   */
  personaOutOfDate: boolean;
  /**
   * `true` when the agent's linked persona no longer exists. Distinct from
   * out-of-date: there is no current persona to respawn into, so do not prompt
   * a respawn — the pinned snapshot is all the config that remains.
   */
  personaOrphaned: boolean;
  /**
   * `true` when the running process was spawned with a config that no longer
   * matches what a spawn would use today — a plain restart would change what
   * runs. Complements `personaOutOfDate` ("a respawn would change it").
   * Always `false` for stopped agents.
   */
  needsRestart: boolean;
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

/**
 * Inbound author gate mode. Mirrors `buzz-acp`'s `--respond-to` CLI flag.
 * `"nobody"` is supported by the harness but not surfaced through this API —
 * it's a heartbeat-only mode without a meaningful GUI use case.
 */
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
  /** Inbound author gate mode. Omitted = `"owner-only"` (server default). */
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

/**
 * Outcome of a live `switch_model` control frame, surfaced asynchronously via
 * the agent's `control_result` observer frame. Busy path: `sent` (cancel +
 * requeue on the new model) or `turn_ending` (oneshot already consumed this
 * turn). Idle path: `switched`, `unsupported_model`, or `no_active_turn`.
 */
export type SwitchManagedAgentModelStatus =
  | "sent"
  | "turn_ending"
  | "switched"
  | "unsupported_model"
  | "no_active_turn";

export type ControlResultFrame = {
  type: "cancel_turn" | "switch_model";
  status: string;
  modelId?: string;
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
  installHint: string;
  installInstructionsUrl: string;
  canAutoInstall: boolean;
  underlyingCliPath: string | null;
  /** True when an npm adapter step is pending but Node.js / npm is absent. */
  nodeRequired: boolean;
  /** Login/auth status for CLI-based runtimes. */
  authStatus: AuthStatus;
  /** Hint for completing authentication; null when not applicable or already logged in. */
  loginHint: string | null;
};

/** An AcpRuntimeCatalogEntry that is confirmed available — command and binaryPath are non-null. */
export type AcpRuntime = AcpRuntimeCatalogEntry & {
  availability: "available";
  command: string;
  binaryPath: string;
};

export type InstallStepResult = {
  step: string;
  command: string;
  success: boolean;
  stdout: string;
  stderr: string;
  exitCode: number | null;
  hint?: string;
};

export type InstallRuntimeResult = {
  success: boolean;
  steps: InstallStepResult[];
  restartedCount: number;
  failedRestartCount: number;
};

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
  | "harnessConstraint";

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
export type AgentPersona = {
  id: string;
  displayName: string;
  avatarUrl: string | null;
  systemPrompt: string;
  /** Preferred ACP runtime ID (e.g. "goose", "claude"). */
  runtime: string | null;
  /** Opaque, harness-specific model identifier string. Buzz stores and passes through without interpretation. */
  model: string | null;
  /** LLM inference provider (e.g. "databricks", "anthropic"). Injected as the runtime's provider env var at spawn time. */
  provider: string | null;
  namePool: string[];
  isBuiltIn: boolean;
  isActive: boolean;
  /** Team ID if this persona was imported from a team directory. Team personas are non-editable. */
  sourceTeam?: string | null;
  /** Environment variables injected for agents created from this persona.
   * Layered as: desktop parent env < persona envVars < agent envVars. */
  envVars: Record<string, string>;
  /** NIP-AP behavioral defaults (wire shape). Null/empty = unset. */
  respondTo: RespondToMode | null;
  respondToAllowlist: string[];
  parallelism: number | null;
  createdAt: string;
  updatedAt: string;
};

/**
 * NIP-AP behavioral group for a definition, sent as one group: absent = don't
 * touch the stored behavior group (legacy callers), present = replace the fields as a
 * unit. Mirrors `PersonaBehaviorRequest`.
 */
export type PersonaBehaviorInput = {
  respondTo?: RespondToMode;
  respondToAllowlist?: string[];
  parallelism?: number;
};

export type CreatePersonaInput = {
  displayName: string;
  avatarUrl?: string;
  systemPrompt: string;
  runtime?: string;
  model?: string;
  provider?: string;
  namePool?: string[];
  envVars?: Record<string, string>;
  behavior?: PersonaBehaviorInput;
};

export type UpdatePersonaInput = {
  id: string;
  displayName: string;
  avatarUrl?: string;
  systemPrompt: string;
  runtime?: string;
  model?: string;
  provider?: string;
  namePool?: string[];
  envVars?: Record<string, string>;
  behavior?: PersonaBehaviorInput;
};

// ── Team types ────────────────────────────────────────────────────────────────
export type AgentTeam = {
  id: string;
  name: string;
  description: string | null;
  instructions: string | null;
  personaIds: string[];
  isBuiltin: boolean;
  /** Absolute path to the team's backing directory (if directory-backed). */
  sourceDir: string | null;
  /** Whether sourceDir is a symlink to an external directory. */
  isSymlink: boolean;
  /** Resolved symlink target path (for display). Only set when isSymlink is true. */
  symlinkTarget: string | null;
  /** Version from the team's plugin.json manifest. */
  version: string | null;
  createdAt: string;
  updatedAt: string;
};

export type CreateTeamInput = {
  name: string;
  description?: string;
  instructions?: string;
  personaIds: string[];
};

export type UpdateTeamInput = {
  id: string;
  name: string;
  description?: string;
  instructions?: string;
  personaIds: string[];
};
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
