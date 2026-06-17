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
};

export type Profile = {
  pubkey: string;
  displayName: string | null;
  avatarUrl: string | null;
  about: string | null;
  nip05Handle: string | null;
};

export type UserProfileSummary = {
  displayName: string | null;
  avatarUrl: string | null;
  nip05Handle: string | null;
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
  isAgent: boolean;
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
};

export type ManagedAgentBackend =
  | { type: "local" }
  | { type: "provider"; id: string; config: Record<string, unknown> };

export type ManagedAgent = {
  pubkey: string;
  name: string;
  personaId: string | null;
  relayUrl: string;
  acpCommand: string;
  agentCommand: string;
  agentArgs: string[];
  mcpCommand: string;
  turnTimeoutSeconds: number;
  idleTimeoutSeconds: number | null;
  maxTurnDurationSeconds: number | null;
  parallelism: number;
  systemPrompt: string | null;
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
  mcpToolsets: string | null;
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
  logPath: string;
  startOnAppLaunch: boolean;
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
  relayUrl?: string;
  acpCommand?: string;
  agentCommand?: string;
  agentArgs?: string[];
  mcpCommand?: string;
  turnTimeoutSeconds?: number;
  idleTimeoutSeconds?: number;
  maxTurnDurationSeconds?: number;
  parallelism?: number;
  systemPrompt?: string;
  avatarUrl?: string;
  model?: string;
  mcpToolsets?: string;
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

export type AcpAvailabilityStatus =
  | "available"
  | "adapter_missing"
  | "cli_missing"
  | "not_installed";

export type AcpRuntimeCatalogEntry = {
  id: string;
  label: string;
  avatarUrl: string;
  availability: AcpAvailabilityStatus;
  command: string | null;
  binaryPath: string | null;
  defaultArgs: string[];
  mcpCommand: string | null;
  installHint: string;
  installInstructionsUrl: string;
  canAutoInstall: boolean;
  underlyingCliPath: string | null;
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
};

export type InstallRuntimeResult = {
  success: boolean;
  steps: InstallStepResult[];
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
export type UpdateManagedAgentInput = {
  pubkey: string;
  name?: string;
  model?: string | null;
  systemPrompt?: string | null;
  mcpToolsets?: string | null;
  /** Absent = don't touch. Present = replace the env_vars map entirely. */
  envVars?: Record<string, string>;
  parallelism?: number;
  turnTimeoutSeconds?: number;
  relayUrl?: string;
  acpCommand?: string;
  agentCommand?: string;
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
  createdAt: string;
  updatedAt: string;
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
};

// ── Team types ────────────────────────────────────────────────────────────────
export type AgentTeam = {
  id: string;
  name: string;
  description: string | null;
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
  personaIds: string[];
};

export type UpdateTeamInput = {
  id: string;
  name: string;
  description?: string;
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
