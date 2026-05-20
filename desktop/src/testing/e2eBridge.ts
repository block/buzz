import { hexToBytes } from "@noble/hashes/utils.js";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { finalizeEvent } from "nostr-tools/pure";
import { parse as yamlParse } from "yaml";

import type { RelayEvent } from "@/shared/api/types";

type TestIdentity = {
  privateKey: string;
  pubkey: string;
  username: string;
};

type MockAcpProvider = {
  id: string;
  label: string;
  command: string;
  binaryPath: string;
  defaultArgs: string[];
  mcpCommand?: string | null;
};

type MockCommandAvailability = {
  available?: boolean;
  command?: string;
  resolvedPath?: string | null;
};

type E2eConfig = {
  mode?: "mock" | "relay";
  mock?: {
    acpProviders?: MockAcpProvider[];
    managedAgentPrereqs?: {
      acp?: MockCommandAvailability;
      mcp?: MockCommandAvailability;
    };
    profileReadDelayMs?: number;
    profileReadError?: string;
    profileUpdateError?: string;
  };
  relayHttpUrl?: string;
  relayWsUrl?: string;
  identity?: TestIdentity;
};

type RawRelayMember = {
  pubkey: string;
  role: "owner" | "admin" | "member";
  added_by: string | null;
  created_at: string;
};

type RawProfile = {
  pubkey: string;
  display_name: string | null;
  avatar_url: string | null;
  about: string | null;
  nip05_handle: string | null;
};

type RawUserProfileSummary = {
  display_name: string | null;
  avatar_url: string | null;
  nip05_handle: string | null;
};

type RawUsersBatchResponse = {
  profiles: Record<string, RawUserProfileSummary>;
  missing: string[];
};

type RawUserSearchResult = {
  pubkey: string;
  display_name: string | null;
  avatar_url: string | null;
  nip05_handle: string | null;
};

type RawSearchUsersResponse = {
  users: RawUserSearchResult[];
};

type PresenceStatus = "online" | "away" | "offline";

type RawPresenceLookup = Record<string, PresenceStatus>;

type RawSetPresenceResponse = {
  status: PresenceStatus;
  ttl_seconds: number;
};

type RawChannel = {
  id: string;
  name: string;
  channel_type: "stream" | "forum" | "dm";
  visibility: "open" | "private";
  description: string;
  topic: string | null;
  purpose: string | null;
  member_count: number;
  last_message_at: string | null;
  archived_at: string | null;
  participants: string[];
  participant_pubkeys: string[];
  ttl_seconds: number | null;
  ttl_deadline: string | null;
};

type RawChannelWithMembership = RawChannel & {
  is_member: boolean;
};

type RawChannelDetail = RawChannel & {
  created_by: string;
  created_at: string;
  updated_at: string;
  topic_set_by: string | null;
  topic_set_at: string | null;
  purpose_set_by: string | null;
  purpose_set_at: string | null;
  topic_required: boolean;
  max_members: number | null;
  nip29_group_id: string | null;
};

type RawChannelMember = {
  pubkey: string;
  role: "owner" | "admin" | "member" | "guest" | "bot";
  joined_at: string;
  display_name: string | null;
};

type RawChannelMembersResponse = {
  members: RawChannelMember[];
  next_cursor: string | null;
};

type RawAddChannelMembersResponse = {
  added: string[];
  errors: Array<{
    pubkey: string;
    error: string;
  }>;
};

type MockChannel = RawChannelDetail & {
  members: RawChannelMember[];
};

type RawFeedItem = {
  id: string;
  kind: number;
  pubkey: string;
  content: string;
  created_at: number;
  channel_id: string | null;
  channel_name: string;
  channel_type?: string;
  tags: string[][];
  category: "mention" | "needs_action" | "activity" | "agent_activity";
};

type RawHomeFeedResponse = {
  feed: {
    mentions: RawFeedItem[];
    needs_action: RawFeedItem[];
    activity: RawFeedItem[];
    agent_activity: RawFeedItem[];
  };
  meta: {
    since: number;
    total: number;
    generated_at: number;
  };
};

type RawThreadSummary = {
  reply_count: number;
  descendant_count: number;
  last_reply_at: number | null;
  participants: string[];
};

type RawForumPost = {
  event_id: string;
  pubkey: string;
  content: string;
  kind: number;
  created_at: number;
  channel_id: string;
  tags: string[][];
  thread_summary: RawThreadSummary | null;
  reactions: unknown;
};

type RawForumPostsResponse = {
  messages: RawForumPost[];
  next_cursor: number | null;
};

type RawForumReply = {
  event_id: string;
  pubkey: string;
  content: string;
  kind: number;
  created_at: number;
  channel_id: string;
  tags: string[][];
  parent_event_id: string | null;
  root_event_id: string | null;
  depth: number;
  broadcast: boolean;
  reactions: unknown;
};

type RawForumThreadResponse = {
  root: RawForumPost;
  replies: RawForumReply[];
  total_replies: number;
  next_cursor: string | null;
};

type RawUserNote = {
  id: string;
  pubkey: string;
  created_at: number;
  content: string;
};

type RawUserNotesCursor = {
  before: number;
  before_id: string;
};

type RawUserNotesResponse = {
  notes: RawUserNote[];
  next_cursor: RawUserNotesCursor | null;
};

type RawSearchHit = {
  event_id: string;
  content: string;
  kind: number;
  pubkey: string;
  channel_id: string | null;
  channel_name: string | null;
  created_at: number;
  score: number;
};

type RawSearchResponse = {
  hits: RawSearchHit[];
  found: number;
};

type RawSendChannelMessageResponse = {
  event_id: string;
  parent_event_id: string | null;
  root_event_id: string | null;
  depth: number;
  created_at: number;
};

type RawRelayAgent = {
  pubkey: string;
  name: string;
  agent_type: string;
  channels: string[];
  channel_ids: string[];
  capabilities: string[];
  status: PresenceStatus;
};

type RawManagedAgent = {
  pubkey: string;
  name: string;
  persona_id: string | null;
  relay_url: string;
  acp_command: string;
  agent_command: string;
  agent_args: string[];
  mcp_command: string;
  turn_timeout_seconds: number;
  idle_timeout_seconds: number | null;
  max_turn_duration_seconds: number | null;
  parallelism: number;
  system_prompt: string | null;
  model: string | null;
  env_vars?: Record<string, string>;
  status: "running" | "stopped" | "deployed" | "not_deployed";
  pid: number | null;
  created_at: string;
  updated_at: string;
  last_started_at: string | null;
  last_stopped_at: string | null;
  last_exit_code: number | null;
  last_error: string | null;
  log_path: string;
  start_on_app_launch: boolean;
  backend:
    | { type: "local" }
    | { type: "provider"; id: string; config: Record<string, unknown> };
  backend_agent_id: string | null;
  respond_to: "owner-only" | "allowlist" | "anyone";
  respond_to_allowlist: string[];
};

type RawCreateManagedAgentResponse = {
  agent: RawManagedAgent;
  private_key_nsec: string;
  profile_sync_error: string | null;
  spawn_error: string | null;
};

type RawManagedAgentLog = {
  content: string;
  log_path: string;
};

type RawAcpProvider = {
  id: string;
  label: string;
  command: string;
  binary_path: string;
  default_args: string[];
  mcp_command: string | null;
};

type RawCommandAvailability = {
  command: string;
  resolved_path: string | null;
  available: boolean;
};

type RawManagedAgentPrereqs = {
  acp: RawCommandAvailability;
  mcp: RawCommandAvailability;
};

type RawPersona = {
  id: string;
  display_name: string;
  avatar_url: string | null;
  system_prompt: string;
  is_builtin: boolean;
  is_active: boolean;
  env_vars?: Record<string, string>;
  created_at: string;
  updated_at: string;
};

type RawTeam = {
  id: string;
  name: string;
  description: string | null;
  persona_ids: string[];
  is_builtin: boolean;
  created_at: string;
  updated_at: string;
};

type MockManagedAgent = RawManagedAgent & {
  private_key_nsec: string;
  log_lines: string[];
};

type WsHandler = (message: unknown) => void;
const GLOBAL_MOCK_SUBSCRIPTION = "*";

type MockSubscription = {
  channelId: string;
  kinds: number[] | null;
};

type MockSocket = {
  handler: WsHandler;
  subscriptions: Map<string, MockSubscription>;
};

function createMockRelayMembershipEvent(): RelayEvent {
  return createMockEvent(
    13534,
    "",
    mockRelayMembers.map((member) => ["member", member.pubkey, member.role]),
    "f".repeat(64),
  );
}

function updateMockRelayMembershipFromAdminEvent(event: RelayEvent): boolean {
  const targetPubkey = event.tags
    .find((tag) => tag[0] === "p")?.[1]
    ?.toLowerCase();
  if (!targetPubkey) return false;

  if (event.kind === 9030) {
    const role = event.tags.find((tag) => tag[0] === "role")?.[1] ?? "member";
    if (role !== "admin" && role !== "member") return false;
    if (mockRelayMembers.some((member) => member.pubkey === targetPubkey)) {
      return true;
    }
    mockRelayMembers.push({
      pubkey: targetPubkey,
      role,
      added_by: event.pubkey,
      created_at: new Date().toISOString(),
    });
    return true;
  }

  if (event.kind === 9031) {
    mockRelayMembers = mockRelayMembers.filter(
      (member) => member.pubkey !== targetPubkey,
    );
    return true;
  }

  if (event.kind === 9032) {
    const role = event.tags.find((tag) => tag[0] === "role")?.[1];
    if (role !== "admin" && role !== "member") return false;
    mockRelayMembers = mockRelayMembers.map((member) =>
      member.pubkey === targetPubkey ? { ...member, role } : member,
    );
    return true;
  }

  return false;
}

declare global {
  interface Window {
    __SPROUT_E2E__?: E2eConfig;
    __SPROUT_E2E_COMMANDS__?: string[];
    __SPROUT_E2E_WEBVIEW_ZOOM__?: number;
    __SPROUT_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
      channelName: string;
      kind?: number;
    }) => boolean;
    __SPROUT_E2E_EMIT_MOCK_MESSAGE__?: (input: {
      channelName: string;
      content: string;
      parentEventId?: string | null;
      pubkey?: string;
      kind?: number;
    }) => RelayEvent;
    __SPROUT_E2E_EMIT_MOCK_TYPING__?: (input: {
      channelName: string;
      pubkey?: string;
    }) => RelayEvent;
    __SPROUT_E2E_INVOKE_MOCK_COMMAND__?: (
      command: string,
      payload?: Record<string, unknown>,
    ) => Promise<unknown>;
    __SPROUT_E2E_PUSH_MOCK_FEED_ITEM__?: (item: RawFeedItem) => RawFeedItem;
  }
}

const DEFAULT_RELAY_HTTP_URL = "http://localhost:3000";
const DEFAULT_RELAY_WS_URL = "ws://localhost:3000";
const E2E_IDENTITY_OVERRIDE_STORAGE_KEY = "sprout:e2e-identity-override.v1";
const DEFAULT_MOCK_IDENTITY = {
  pubkey: "deadbeef".repeat(8),
  display_name: "npub1mock...",
};
const DEFAULT_REAL_IDENTITY = {
  privateKey:
    "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03",
  pubkey: "e5ebc6cdb579be112e336cc319b5989b4bb6af11786ea90dbe52b5f08d741b34",
  username: "tyler",
} satisfies TestIdentity;

const ALICE_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
const BOB_PUBKEY =
  "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260";
const CHARLIE_PUBKEY =
  "554cef57437abac34522ac2c9f0490d685b72c80478cf9f7ed6f9570ee8624ea";
const OUTSIDER_PUBKEY =
  "df8e91b86fda13a9a67896df77232f7bdab2ba9c3e165378e1ba3d24c13a328e";
const MOCK_IDENTITY_PUBKEY = DEFAULT_MOCK_IDENTITY.pubkey;
const MOCK_PRESENCE_TTL_SECONDS = 90;

const mockDisplayNames = new Map<string, string>([
  [MOCK_IDENTITY_PUBKEY, DEFAULT_MOCK_IDENTITY.display_name],
  [ALICE_PUBKEY, "alice"],
  [BOB_PUBKEY, "bob"],
  [CHARLIE_PUBKEY, "charlie"],
  [OUTSIDER_PUBKEY, "outsider"],
  [DEFAULT_REAL_IDENTITY.pubkey, DEFAULT_REAL_IDENTITY.username],
]);

function isoMinutesAgo(minutesAgo: number): string {
  return new Date(Date.now() - minutesAgo * 60_000).toISOString();
}

function cloneMembers(members: RawChannelMember[]): RawChannelMember[] {
  return members.map((member) => ({ ...member }));
}

function toRawChannel(
  channel: MockChannel,
  config?: E2eConfig,
): RawChannelWithMembership {
  const currentPubkey = getMockMemberPubkey(config).toLowerCase();

  return {
    id: channel.id,
    name: channel.name,
    channel_type: channel.channel_type,
    visibility: channel.visibility,
    description: channel.description,
    topic: channel.topic,
    purpose: channel.purpose,
    member_count: channel.member_count,
    last_message_at: channel.last_message_at,
    archived_at: channel.archived_at,
    participants: [...channel.participants],
    participant_pubkeys: [...channel.participant_pubkeys],
    ttl_seconds: channel.ttl_seconds ?? null,
    ttl_deadline: channel.ttl_deadline ?? null,
    is_member: channel.members.some(
      (member) => member.pubkey.toLowerCase() === currentPubkey,
    ),
  };
}

function toRawChannelDetail(
  channel: MockChannel,
  config?: E2eConfig,
): RawChannelDetail {
  return {
    ...toRawChannel(channel, config),
    created_by: channel.created_by,
    created_at: channel.created_at,
    updated_at: channel.updated_at,
    topic_set_by: channel.topic_set_by,
    topic_set_at: channel.topic_set_at,
    purpose_set_by: channel.purpose_set_by,
    purpose_set_at: channel.purpose_set_at,
    topic_required: channel.topic_required,
    max_members: channel.max_members,
    nip29_group_id: channel.nip29_group_id,
  };
}

function createMockMember(
  pubkey: string,
  role: RawChannelMember["role"],
  joinedMinutesAgo: number,
): RawChannelMember {
  return {
    pubkey,
    role,
    joined_at: isoMinutesAgo(joinedMinutesAgo),
    display_name: mockDisplayNames.get(pubkey) ?? null,
  };
}

function createMockChannel(
  seed: Omit<
    MockChannel,
    | "created_at"
    | "member_count"
    | "members"
    | "updated_at"
    | "participant_pubkeys"
    | "participants"
    | "ttl_seconds"
    | "ttl_deadline"
  > & {
    created_minutes_ago: number;
    members: RawChannelMember[];
    participant_pubkeys?: string[];
    participants?: string[];
    ttl_seconds?: number | null;
    ttl_deadline?: string | null;
    updated_minutes_ago?: number;
  },
): MockChannel {
  return {
    ...seed,
    created_at: isoMinutesAgo(seed.created_minutes_ago),
    member_count: seed.members.length,
    members: cloneMembers(seed.members),
    participant_pubkeys: [...(seed.participant_pubkeys ?? [])],
    participants: [...(seed.participants ?? [])],
    ttl_seconds: seed.ttl_seconds ?? null,
    ttl_deadline: seed.ttl_deadline ?? null,
    updated_at: isoMinutesAgo(
      seed.updated_minutes_ago ?? seed.created_minutes_ago,
    ),
  };
}

function syncMockChannel(channel: MockChannel) {
  channel.member_count = channel.members.length;

  if (channel.channel_type !== "dm") {
    return;
  }

  channel.participant_pubkeys = channel.members.map((member) => member.pubkey);
  channel.participants = channel.members.map(
    (member) => member.display_name ?? member.pubkey.slice(0, 8),
  );
}

function touchMockChannel(channel: MockChannel) {
  channel.updated_at = new Date().toISOString();
}

function normalizeParticipantPubkeys(pubkeys: string[]) {
  return [...new Set(pubkeys.map((pubkey) => pubkey.toLowerCase()))].sort();
}

function findMockDmByParticipantPubkeys(pubkeys: string[]) {
  const normalizedPubkeys = normalizeParticipantPubkeys(pubkeys);

  return (
    mockChannels.find((channel) => {
      if (channel.channel_type !== "dm") {
        return false;
      }

      const channelPubkeys = normalizeParticipantPubkeys(
        channel.participant_pubkeys,
      );

      return (
        channelPubkeys.length === normalizedPubkeys.length &&
        channelPubkeys.every(
          (pubkey, index) => pubkey === normalizedPubkeys[index],
        )
      );
    }) ?? null
  );
}

function getMockIdentity() {
  return {
    pubkey: MOCK_IDENTITY_PUBKEY,
    displayName: DEFAULT_MOCK_IDENTITY.display_name,
  };
}

function cloneProfile(profile: RawProfile): RawProfile {
  return { ...profile };
}

function cloneRelayAgent(agent: RawRelayAgent): RawRelayAgent {
  return {
    ...agent,
    channels: [...agent.channels],
    channel_ids: [...agent.channel_ids],
    capabilities: [...agent.capabilities],
  };
}

function cloneManagedAgent(agent: MockManagedAgent): RawManagedAgent {
  return {
    pubkey: agent.pubkey,
    name: agent.name,
    persona_id: agent.persona_id,
    relay_url: agent.relay_url,
    acp_command: agent.acp_command,
    agent_command: agent.agent_command,
    agent_args: [...agent.agent_args],
    mcp_command: agent.mcp_command,
    turn_timeout_seconds: agent.turn_timeout_seconds,
    idle_timeout_seconds: agent.idle_timeout_seconds ?? null,
    max_turn_duration_seconds: agent.max_turn_duration_seconds ?? null,
    parallelism: agent.parallelism,
    system_prompt: agent.system_prompt,
    model: agent.model,
    env_vars: { ...(agent.env_vars ?? {}) },
    status: agent.status,
    pid: agent.pid,
    created_at: agent.created_at,
    updated_at: agent.updated_at,
    last_started_at: agent.last_started_at,
    last_stopped_at: agent.last_stopped_at,
    last_exit_code: agent.last_exit_code,
    last_error: agent.last_error,
    log_path: agent.log_path,
    start_on_app_launch: agent.start_on_app_launch,
    backend: agent.backend ?? { type: "local" as const },
    backend_agent_id: agent.backend_agent_id ?? null,
    respond_to: agent.respond_to ?? "owner-only",
    respond_to_allowlist: agent.respond_to_allowlist
      ? [...agent.respond_to_allowlist]
      : [],
  };
}

function resetMockRelayMembers(config: E2eConfig | undefined) {
  const pubkey = getMockMemberPubkey(config);
  mockRelayMembers = [
    {
      pubkey,
      role: "owner",
      added_by: null,
      created_at: isoMinutesAgo(120),
    },
    {
      pubkey: ALICE_PUBKEY,
      role: "admin",
      added_by: pubkey,
      created_at: isoMinutesAgo(90),
    },
    {
      pubkey: BOB_PUBKEY,
      role: "member",
      added_by: pubkey,
      created_at: isoMinutesAgo(60),
    },
  ];
}

function resetMockManagedAgents() {
  mockManagedAgents = [];
  syncMockRelayAgentsFromManagedAgents();
}

function resetMockPersonas() {
  const now = new Date().toISOString();
  mockPersonas = [
    {
      id: "builtin:solo",
      display_name: "Solo",
      avatar_url: null,
      system_prompt: "You are Solo.",
      is_builtin: true,
      is_active: false,
      created_at: now,
      updated_at: now,
    },
    {
      id: "builtin:kit",
      display_name: "Kit",
      avatar_url: null,
      system_prompt: "You are Kit.",
      is_builtin: true,
      is_active: false,
      created_at: now,
      updated_at: now,
    },
    {
      id: "builtin:scout",
      display_name: "Scout",
      avatar_url: null,
      system_prompt: "You are Scout.",
      is_builtin: true,
      is_active: false,
      created_at: now,
      updated_at: now,
    },
  ];
}

function resetMockTeams() {
  mockTeams = [];
}

function getMockProfileByPubkey(pubkey: string): RawProfile | null {
  const normalizedPubkey = pubkey.toLowerCase();
  const existing = mockProfiles.get(normalizedPubkey);
  if (existing) {
    return existing;
  }

  if (!mockDisplayNames.has(normalizedPubkey)) {
    return null;
  }

  return {
    pubkey: normalizedPubkey,
    display_name: mockDisplayNames.get(normalizedPubkey) ?? null,
    avatar_url: null,
    about: null,
    nip05_handle: null,
  };
}

function listMockProfiles(): RawProfile[] {
  const pubkeys = new Set<string>([
    ...mockProfiles.keys(),
    ...mockDisplayNames.keys(),
    DEFAULT_REAL_IDENTITY.pubkey,
  ]);

  return [...pubkeys]
    .map((pubkey) => getMockProfileByPubkey(pubkey))
    .filter((profile): profile is RawProfile => profile !== null);
}

function listMockChannels(config?: E2eConfig): RawChannelWithMembership[] {
  return mockChannels.map((channel) => toRawChannel(channel, config));
}

function getMockChannel(channelId: string): MockChannel {
  const channel = mockChannels.find((candidate) => candidate.id === channelId);
  if (!channel) {
    throw new Error(`Channel ${channelId} not found.`);
  }

  return channel;
}

function getMockMemberPubkey(config: E2eConfig | undefined): string {
  return getActiveIdentity(config)?.pubkey ?? getMockIdentity().pubkey;
}

function getMockMemberDisplayName(config: E2eConfig | undefined): string {
  return getActiveIdentity(config)?.username ?? getMockIdentity().displayName;
}

function createCurrentMember(
  config: E2eConfig | undefined,
  role: RawChannelMember["role"],
): RawChannelMember {
  return {
    pubkey: getMockMemberPubkey(config),
    role,
    joined_at: new Date().toISOString(),
    display_name: getMockMemberDisplayName(config),
  };
}

const mockChannels: MockChannel[] = [
  createMockChannel({
    id: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
    name: "general",
    channel_type: "stream",
    visibility: "open",
    description: "General discussion for everyone",
    topic: "Company-wide updates",
    purpose: "Coordinate day-to-day work and unblock the team.",
    last_message_at: isoMinutesAgo(5),
    archived_at: null,
    created_by: MOCK_IDENTITY_PUBKEY,
    topic_set_by: MOCK_IDENTITY_PUBKEY,
    topic_set_at: isoMinutesAgo(90),
    purpose_set_by: MOCK_IDENTITY_PUBKEY,
    purpose_set_at: isoMinutesAgo(80),
    topic_required: false,
    max_members: null,
    nip29_group_id: null,
    created_minutes_ago: 1440,
    updated_minutes_ago: 5,
    members: [
      createMockMember(MOCK_IDENTITY_PUBKEY, "owner", 1440),
      createMockMember(ALICE_PUBKEY, "admin", 1200),
      createMockMember(BOB_PUBKEY, "member", 960),
    ],
  }),
  createMockChannel({
    id: "9dae0116-799b-5071-a0a8-fdd30a91a35d",
    name: "random",
    channel_type: "stream",
    visibility: "open",
    description: "Off-topic, fun stuff",
    topic: null,
    purpose: null,
    last_message_at: null,
    archived_at: null,
    created_by: ALICE_PUBKEY,
    topic_set_by: null,
    topic_set_at: null,
    purpose_set_by: null,
    purpose_set_at: null,
    topic_required: false,
    max_members: null,
    nip29_group_id: null,
    created_minutes_ago: 1400,
    updated_minutes_ago: 1400,
    members: [
      createMockMember(ALICE_PUBKEY, "owner", 1400),
      createMockMember(MOCK_IDENTITY_PUBKEY, "member", 1300),
      createMockMember(BOB_PUBKEY, "member", 1000),
    ],
  }),
  createMockChannel({
    id: "b5e2f8a1-3c44-5912-9e67-4a8d1f2b3c4e",
    name: "design",
    channel_type: "stream",
    visibility: "open",
    description: "Design system and UX discussions with engineering partners",
    topic: null,
    purpose: null,
    last_message_at: isoMinutesAgo(120),
    archived_at: null,
    created_by: ALICE_PUBKEY,
    topic_set_by: null,
    topic_set_at: null,
    purpose_set_by: null,
    purpose_set_at: null,
    topic_required: false,
    max_members: null,
    nip29_group_id: null,
    created_minutes_ago: 1350,
    updated_minutes_ago: 120,
    members: [
      createMockMember(ALICE_PUBKEY, "owner", 1350),
      createMockMember(BOB_PUBKEY, "member", 1100),
    ],
  }),
  createMockChannel({
    id: "c6f3a9b2-4d55-5a23-bf78-5b9e2g3c5d6f",
    name: "sales",
    channel_type: "stream",
    visibility: "open",
    description: "Sales team coordination and pipeline updates",
    topic: "Q1 targets",
    purpose: null,
    last_message_at: isoMinutesAgo(30),
    archived_at: null,
    created_by: BOB_PUBKEY,
    topic_set_by: BOB_PUBKEY,
    topic_set_at: isoMinutesAgo(200),
    purpose_set_by: null,
    purpose_set_at: null,
    topic_required: false,
    max_members: null,
    nip29_group_id: null,
    created_minutes_ago: 1300,
    updated_minutes_ago: 30,
    members: [
      createMockMember(BOB_PUBKEY, "owner", 1300),
      createMockMember(CHARLIE_PUBKEY, "member", 900),
    ],
  }),
  createMockChannel({
    id: "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9",
    name: "engineering",
    channel_type: "stream",
    visibility: "open",
    description: "Engineering discussions",
    topic: "Desktop release train",
    purpose: "Track implementation details and release readiness.",
    last_message_at: isoMinutesAgo(42),
    archived_at: null,
    created_by: ALICE_PUBKEY,
    topic_set_by: ALICE_PUBKEY,
    topic_set_at: isoMinutesAgo(120),
    purpose_set_by: ALICE_PUBKEY,
    purpose_set_at: isoMinutesAgo(130),
    topic_required: false,
    max_members: null,
    nip29_group_id: null,
    created_minutes_ago: 1320,
    updated_minutes_ago: 42,
    members: [
      createMockMember(ALICE_PUBKEY, "owner", 1320),
      createMockMember(MOCK_IDENTITY_PUBKEY, "member", 1180),
      createMockMember(BOB_PUBKEY, "member", 900),
    ],
  }),
  createMockChannel({
    id: "94a444a4-c0a3-5966-ab05-530c6ddc2301",
    name: "agents",
    channel_type: "stream",
    visibility: "open",
    description: "AI agent testing and collaboration",
    topic: "Coordination board",
    purpose: "Track agent work and relay activity.",
    last_message_at: isoMinutesAgo(15),
    archived_at: null,
    created_by: MOCK_IDENTITY_PUBKEY,
    topic_set_by: MOCK_IDENTITY_PUBKEY,
    topic_set_at: isoMinutesAgo(60),
    purpose_set_by: MOCK_IDENTITY_PUBKEY,
    purpose_set_at: isoMinutesAgo(65),
    topic_required: false,
    max_members: null,
    nip29_group_id: null,
    created_minutes_ago: 1000,
    updated_minutes_ago: 15,
    members: [
      createMockMember(MOCK_IDENTITY_PUBKEY, "owner", 1000),
      createMockMember(CHARLIE_PUBKEY, "bot", 800),
    ],
  }),
  createMockChannel({
    id: "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11",
    name: "watercooler",
    channel_type: "forum",
    visibility: "open",
    description: "Casual forum for async discussions",
    topic: null,
    purpose: null,
    last_message_at: null,
    archived_at: null,
    created_by: ALICE_PUBKEY,
    topic_set_by: null,
    topic_set_at: null,
    purpose_set_by: null,
    purpose_set_at: null,
    topic_required: false,
    max_members: null,
    nip29_group_id: null,
    created_minutes_ago: 900,
    updated_minutes_ago: 900,
    members: [
      createMockMember(ALICE_PUBKEY, "owner", 900),
      createMockMember(MOCK_IDENTITY_PUBKEY, "member", 750),
    ],
  }),
  createMockChannel({
    id: "1be1dcdb-4c31-5a8c-81de-ac102552ca10",
    name: "announcements",
    channel_type: "forum",
    visibility: "private",
    description: "Company announcements",
    topic: "Leadership updates",
    purpose: "Read-only announcements for the workspace.",
    last_message_at: null,
    archived_at: null,
    created_by: ALICE_PUBKEY,
    topic_set_by: ALICE_PUBKEY,
    topic_set_at: isoMinutesAgo(200),
    purpose_set_by: ALICE_PUBKEY,
    purpose_set_at: isoMinutesAgo(210),
    topic_required: false,
    max_members: null,
    nip29_group_id: null,
    created_minutes_ago: 880,
    updated_minutes_ago: 200,
    members: [
      createMockMember(ALICE_PUBKEY, "owner", 880),
      createMockMember(MOCK_IDENTITY_PUBKEY, "guest", 700),
    ],
  }),
  createMockChannel({
    id: "f48efb06-0c93-5025-aac9-2e646bb6bfa8",
    name: "alice-tyler",
    channel_type: "dm",
    visibility: "private",
    description: "DM between alice and tyler",
    topic: null,
    purpose: null,
    last_message_at: null,
    archived_at: null,
    created_by: ALICE_PUBKEY,
    topic_set_by: null,
    topic_set_at: null,
    purpose_set_by: null,
    purpose_set_at: null,
    topic_required: false,
    max_members: 2,
    nip29_group_id: null,
    created_minutes_ago: 720,
    updated_minutes_ago: 720,
    participants: ["alice", "tyler"],
    participant_pubkeys: [ALICE_PUBKEY, MOCK_IDENTITY_PUBKEY],
    members: [
      createMockMember(ALICE_PUBKEY, "member", 720),
      createMockMember(MOCK_IDENTITY_PUBKEY, "member", 720),
    ],
  }),
  createMockChannel({
    id: "7eb9f239-9393-50b0-bd76-d85eef0511c7",
    name: "bob-tyler",
    channel_type: "dm",
    visibility: "private",
    description: "DM between bob and tyler",
    topic: null,
    purpose: null,
    last_message_at: null,
    archived_at: null,
    created_by: BOB_PUBKEY,
    topic_set_by: null,
    topic_set_at: null,
    purpose_set_by: null,
    purpose_set_at: null,
    topic_required: false,
    max_members: 2,
    nip29_group_id: null,
    created_minutes_ago: 700,
    updated_minutes_ago: 700,
    participants: ["bob", "tyler"],
    participant_pubkeys: [BOB_PUBKEY, MOCK_IDENTITY_PUBKEY],
    members: [
      createMockMember(BOB_PUBKEY, "member", 700),
      createMockMember(MOCK_IDENTITY_PUBKEY, "member", 700),
    ],
  }),
];

const mockMessages = new Map<string, RelayEvent[]>();
let mockRelayMembers: RawRelayMember[] = [];
const mockSockets = new Map<number, MockSocket>();
const realSockets = new Map<number, WebSocket>();
let mockManagedAgents: MockManagedAgent[] = [];
let mockPersonas: RawPersona[] = [];
let mockTeams: RawTeam[] = [];
let mockRelayAgents: RawRelayAgent[] = [
  {
    pubkey: ALICE_PUBKEY,
    name: "alice",
    agent_type: "goose",
    channels: ["general", "agents"],
    channel_ids: [
      "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
      "94a444a4-c0a3-5966-ab05-530c6ddc2301",
    ],
    capabilities: ["search", "summaries", "workflows"],
    status: "online",
  },
  {
    pubkey: CHARLIE_PUBKEY,
    name: "charlie",
    agent_type: "codex",
    channels: ["general"],
    channel_ids: ["9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50"],
    capabilities: ["code", "reviews"],
    status: "away",
  },
];

// ── Workflow mocks ─────────────────────────────────────────────────────────

type MockWorkflow = {
  id: string;
  name: string;
  owner_pubkey: string;
  channel_id: string | null;
  definition: Record<string, unknown>;
  status: "active" | "disabled" | "archived";
  created_at: number;
  updated_at: number;
};

type RawWorkflowTraceEntry = {
  step_id: string;
  status: string;
  output?: Record<string, unknown>;
  started_at?: number | null;
  completed_at?: number | null;
  error?: string | null;
};

type RawWorkflowRun = {
  id: string;
  workflow_id: string;
  status:
    | "pending"
    | "running"
    | "completed"
    | "failed"
    | "cancelled"
    | "waiting_approval";
  current_step: number | null;
  execution_trace: RawWorkflowTraceEntry[];
  started_at: number | null;
  completed_at: number | null;
  error_message: string | null;
  created_at: number;
};

const mockWorkflows: MockWorkflow[] = [];
let mockWorkflowRuns: RawWorkflowRun[] = [];
let mockWorkflowIdCounter = 0;

function resetMockWorkflows() {
  mockWorkflows.length = 0;
  mockWorkflowRuns = [];
  mockWorkflowIdCounter = 0;
}

function parseWorkflowDefinition(
  yamlDefinition: string,
): Record<string, unknown> {
  const parsed = yamlParse(yamlDefinition);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("Workflow definition must be a YAML object");
  }
  return parsed as Record<string, unknown>;
}

function handleGetChannelWorkflows(args: { channelId: string }) {
  return mockWorkflows.filter((w) => w.channel_id === args.channelId);
}

function handleGetWorkflow(args: { workflowId: string }) {
  const workflow = mockWorkflows.find((w) => w.id === args.workflowId);
  if (!workflow) throw new Error(`Workflow ${args.workflowId} not found`);
  return workflow;
}

function handleCreateWorkflow(args: {
  channelId: string;
  yamlDefinition: string;
}) {
  mockWorkflowIdCounter += 1;
  const now = Math.floor(Date.now() / 1000);
  const definition = parseWorkflowDefinition(args.yamlDefinition);
  const name =
    typeof definition.name === "string"
      ? definition.name
      : `workflow_${mockWorkflowIdCounter}`;
  const workflow: MockWorkflow = {
    id: `mock-wf-${mockWorkflowIdCounter}`,
    name,
    owner_pubkey: MOCK_IDENTITY_PUBKEY,
    channel_id: args.channelId,
    definition,
    status: "active",
    created_at: now,
    updated_at: now,
  };
  mockWorkflows.push(workflow);

  const trigger = definition.trigger as Record<string, unknown> | undefined;
  return {
    ...workflow,
    webhook_secret:
      trigger?.on === "webhook"
        ? `mock-webhook-secret-${mockWorkflowIdCounter}`
        : undefined,
  };
}

function handleUpdateWorkflow(args: {
  workflowId: string;
  yamlDefinition: string;
}) {
  const workflow = mockWorkflows.find((w) => w.id === args.workflowId);
  if (!workflow) throw new Error(`Workflow ${args.workflowId} not found`);
  const definition = parseWorkflowDefinition(args.yamlDefinition);
  if (typeof definition.name === "string") workflow.name = definition.name;
  workflow.definition = definition;
  workflow.updated_at = Math.floor(Date.now() / 1000);

  const trigger = definition.trigger as Record<string, unknown> | undefined;
  return {
    ...workflow,
    webhook_secret:
      trigger?.on === "webhook"
        ? `mock-webhook-secret-${workflow.id}`
        : undefined,
  };
}

function handleDeleteWorkflow(args: { workflowId: string }) {
  const index = mockWorkflows.findIndex((w) => w.id === args.workflowId);
  if (index === -1) throw new Error(`Workflow ${args.workflowId} not found`);
  mockWorkflows.splice(index, 1);
  mockWorkflowRuns = mockWorkflowRuns.filter(
    (run) => run.workflow_id !== args.workflowId,
  );
}

function buildMockWorkflowRun(workflow: MockWorkflow): RawWorkflowRun {
  const createdAt = Math.floor(Date.now() / 1000);
  const rawSteps = Array.isArray(workflow.definition.steps)
    ? workflow.definition.steps
    : [];
  const executionTrace = rawSteps.map((candidate, index) => {
    const step =
      candidate && typeof candidate === "object"
        ? (candidate as Record<string, unknown>)
        : {};
    const startedAt = createdAt + index;
    const completedAt = startedAt + 1;
    const output: Record<string, unknown> = {};

    if (typeof step.action === "string") {
      output.action = step.action;
    }
    if (typeof step.name === "string" && step.name.trim().length > 0) {
      output.name = step.name;
    }
    if (typeof step.text === "string" && step.text.trim().length > 0) {
      output.preview = step.text;
    }

    return {
      step_id:
        typeof step.id === "string" && step.id.trim().length > 0
          ? step.id
          : `step_${index + 1}`,
      status: "completed",
      output,
      started_at: startedAt,
      completed_at: completedAt,
      error: null,
    };
  });

  const startedAt =
    executionTrace.length > 0
      ? (executionTrace[0].started_at ?? createdAt)
      : createdAt;
  const lastTraceEntry = executionTrace[executionTrace.length - 1];
  const completedAt =
    executionTrace.length > 0
      ? (lastTraceEntry?.completed_at ?? createdAt)
      : createdAt;

  return {
    id: `mock-run-${Date.now()}`,
    workflow_id: workflow.id,
    status: "completed",
    current_step: null,
    execution_trace: executionTrace,
    started_at: startedAt,
    completed_at: completedAt,
    error_message: null,
    created_at: createdAt,
  };
}

function handleTriggerWorkflow(args: { workflowId: string }) {
  const workflow = mockWorkflows.find((w) => w.id === args.workflowId);
  if (!workflow) throw new Error(`Workflow ${args.workflowId} not found`);
  const run = buildMockWorkflowRun(workflow);
  mockWorkflowRuns = [run, ...mockWorkflowRuns];
  return {
    run_id: run.id,
    workflow_id: workflow.id,
    status: run.status,
  };
}

function handleGetWorkflowRuns(args: {
  limit?: number | null;
  workflowId: string;
}) {
  const runs = mockWorkflowRuns.filter(
    (run) => run.workflow_id === args.workflowId,
  );
  return args.limit ? runs.slice(0, args.limit) : runs;
}

function handleGetRunApprovals(_args: { workflowId: string; runId: string }) {
  return [];
}

const mockProfiles = new Map<string, RawProfile>([
  [
    MOCK_IDENTITY_PUBKEY,
    {
      pubkey: MOCK_IDENTITY_PUBKEY,
      display_name: DEFAULT_MOCK_IDENTITY.display_name,
      avatar_url: null,
      about: null,
      nip05_handle: null,
    },
  ],
]);
const mockPresence = new Map<string, PresenceStatus>([
  [MOCK_IDENTITY_PUBKEY, "offline"],
  [DEFAULT_REAL_IDENTITY.pubkey, "offline"],
  [ALICE_PUBKEY, "online"],
  [BOB_PUBKEY, "away"],
  [CHARLIE_PUBKEY, "online"],
  [OUTSIDER_PUBKEY, "offline"],
]);
const mockFeedOverrides: RawHomeFeedResponse["feed"] = {
  mentions: [],
  needs_action: [],
  activity: [],
  agent_activity: [],
};

let installed = false;
let nextSocketId = 1;

function syncMockRelayAgentsFromManagedAgents() {
  const baseAgents = mockRelayAgents.filter(
    (agent) =>
      !mockManagedAgents.some((managed) => managed.pubkey === agent.pubkey),
  );
  const managedAgentsAsRelay: RawRelayAgent[] = mockManagedAgents.map(
    (agent) => {
      const memberships = getManagedAgentRelayMembership(agent.pubkey);

      return {
        pubkey: agent.pubkey,
        name: agent.name,
        agent_type: agent.agent_command,
        channels: memberships.channels,
        channel_ids: memberships.channelIds,
        capabilities: ["messages", "channels", "mcp"],
        status:
          agent.status === "running" || agent.status === "deployed"
            ? "online"
            : "offline",
      };
    },
  );

  mockRelayAgents = [...baseAgents, ...managedAgentsAsRelay];
}

function getManagedAgentRelayMembership(pubkey: string) {
  const memberships = mockChannels.filter((channel) =>
    channel.members.some((member) => member.pubkey === pubkey),
  );

  return {
    channelIds: memberships.map((channel) => channel.id),
    channels: memberships.map((channel) => channel.name),
  };
}

function getConfig(): E2eConfig | undefined {
  return window.__SPROUT_E2E__;
}

function readStoredIdentityOverride(): TestIdentity | undefined {
  try {
    const rawValue = window.localStorage.getItem(
      E2E_IDENTITY_OVERRIDE_STORAGE_KEY,
    );
    if (!rawValue) {
      return undefined;
    }

    const parsed = JSON.parse(rawValue);
    if (
      !parsed ||
      typeof parsed !== "object" ||
      typeof parsed.privateKey !== "string" ||
      typeof parsed.pubkey !== "string" ||
      typeof parsed.username !== "string"
    ) {
      return undefined;
    }

    return {
      privateKey: parsed.privateKey,
      pubkey: parsed.pubkey,
      username: parsed.username,
    };
  } catch {
    return undefined;
  }
}

function isRelayMode(config: E2eConfig | undefined): boolean {
  return config?.mode === "relay";
}

function getRelayHttpUrl(config: E2eConfig | undefined): string {
  return config?.relayHttpUrl ?? DEFAULT_RELAY_HTTP_URL;
}

function getRelayWsUrl(config: E2eConfig | undefined): string {
  return config?.relayWsUrl ?? DEFAULT_RELAY_WS_URL;
}

function getIdentity(config: E2eConfig | undefined): TestIdentity | undefined {
  if (!isRelayMode(config)) {
    return undefined;
  }

  return config?.identity ?? DEFAULT_REAL_IDENTITY;
}

function getActiveIdentity(config: E2eConfig | undefined) {
  return readStoredIdentityOverride() ?? getIdentity(config);
}

function ensureMockProfile(config: E2eConfig | undefined): RawProfile {
  const pubkey = getMockMemberPubkey(config);
  const existing = mockProfiles.get(pubkey);
  if (existing) {
    return existing;
  }

  const profile = {
    pubkey,
    display_name: getMockMemberDisplayName(config),
    avatar_url: null,
    about: null,
    nip05_handle: null,
  };
  mockProfiles.set(pubkey, profile);
  return profile;
}

function applyMockDisplayName(pubkey: string, displayName: string | null) {
  if (displayName) {
    mockDisplayNames.set(pubkey, displayName);
  } else {
    mockDisplayNames.delete(pubkey);
  }

  for (const channel of mockChannels) {
    for (const member of channel.members) {
      if (member.pubkey === pubkey) {
        member.display_name = displayName;
      }
    }
    syncMockChannel(channel);
  }
}

function getMockPresenceStatus(pubkey: string): PresenceStatus {
  return mockPresence.get(pubkey.toLowerCase()) ?? "offline";
}

function setMockPresenceStatus(pubkey: string, status: PresenceStatus) {
  mockPresence.set(pubkey.toLowerCase(), status);
}

function resolveHandler(handler: unknown): WsHandler {
  if (typeof handler === "function") {
    return handler as WsHandler;
  }

  if (
    typeof handler === "object" &&
    handler !== null &&
    "onmessage" in handler &&
    typeof handler.onmessage === "function"
  ) {
    return handler.onmessage as WsHandler;
  }

  throw new Error("Invalid websocket message handler.");
}

function sendWsText(handler: WsHandler, payload: unknown[]) {
  handler({
    type: "Text",
    data: JSON.stringify(payload),
  });
}

function sendWsClose(handler: WsHandler) {
  handler({
    type: "Close",
  });
}

function getChannelIdFromTags(tags: string[][]): string | undefined {
  return tags.find((tag) => tag[0] === "h")?.[1];
}

function getThreadReferenceFromTags(tags: string[][]) {
  const eventTags = tags.filter(
    (tag) => tag[0] === "e" && typeof tag[1] === "string",
  );

  if (eventTags.length === 0) {
    return {
      parentEventId: null,
      rootEventId: null,
    };
  }

  const rootTag = eventTags.find((tag) => tag[3] === "root");
  const replyTag =
    [...eventTags].reverse().find((tag) => tag[3] === "reply") ?? null;

  if (!replyTag) {
    return {
      parentEventId: null,
      rootEventId: null,
    };
  }

  return {
    parentEventId: replyTag[1] ?? null,
    rootEventId: rootTag?.[1] ?? replyTag[1] ?? null,
  };
}

function appendMentionTags(
  tags: string[][],
  mentionPubkeys: string[] | undefined,
  selfPubkey: string,
) {
  const selfLower = selfPubkey.toLowerCase();
  const seen = new Set<string>([selfLower]);
  for (const pk of mentionPubkeys ?? []) {
    const lower = pk.toLowerCase();
    if (seen.has(lower)) {
      continue;
    }
    seen.add(lower);
    tags.push(["p", lower]);
  }
}

function buildTopLevelMessageTags(
  channelId: string,
  mentionPubkeys: string[] | undefined,
  selfPubkey: string,
) {
  const tags: string[][] = [["h", channelId]];
  appendMentionTags(tags, mentionPubkeys, selfPubkey);
  return tags;
}

function buildReplyMessageTags(
  channelId: string,
  authorPubkey: string,
  parentEventId: string,
  rootEventId: string,
  mentionPubkeys: string[] | undefined,
) {
  // Preserve the reply tag ordering that the desktop message hooks already
  // expect locally: author p, h, mention ps, then thread e-tags.
  const tags: string[][] = [
    ["p", authorPubkey],
    ["h", channelId],
  ];
  appendMentionTags(tags, mentionPubkeys, authorPubkey);

  if (parentEventId === rootEventId) {
    tags.push(["e", rootEventId, "", "reply"]);
    return tags;
  }

  tags.push(["e", rootEventId, "", "root"]);
  tags.push(["e", parentEventId, "", "reply"]);
  return tags;
}

function getMockMessageStore(channelId: string): RelayEvent[] {
  const existing = mockMessages.get(channelId);
  if (existing) {
    return existing;
  }

  const seeded: RelayEvent[] =
    channelId === "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50"
      ? [
          {
            id: "mock-general-welcome",
            pubkey: DEFAULT_MOCK_IDENTITY.pubkey,
            created_at: Math.floor(Date.now() / 1000),
            kind: 9,
            tags: [["h", channelId]],
            content: "Welcome to #general",
            sig: "mocksig".repeat(20).slice(0, 128),
          },
        ]
      : channelId === "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11"
        ? [
            {
              id: "mock-forum-release-thread",
              pubkey:
                "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
              created_at: Math.floor(Date.now() / 1000) - 90 * 60,
              kind: 45001,
              tags: [["h", channelId]],
              content: "Release checklist: async feedback thread.",
              sig: "mocksig".repeat(20).slice(0, 128),
            },
            {
              id: "mock-forum-release-reply",
              pubkey: ALICE_PUBKEY,
              created_at: Math.floor(Date.now() / 1000) - 80 * 60,
              kind: 45003,
              tags: buildReplyMessageTags(
                channelId,
                ALICE_PUBKEY,
                "mock-forum-release-thread",
                "mock-forum-release-thread",
                undefined,
              ),
              content: "Looks good to me. We should ship it.",
              sig: "mocksig".repeat(20).slice(0, 128),
            },
          ]
        : [];

  mockMessages.set(channelId, seeded);
  return seeded;
}

function emitMockHistory(socket: MockSocket, subId: string, channelId: string) {
  const events = getMockMessageStore(channelId);
  for (const event of events) {
    sendWsText(socket.handler, ["EVENT", subId, event]);
  }
  sendWsText(socket.handler, ["EOSE", subId]);
}

function emitMockLiveEvent(channelId: string, event: RelayEvent) {
  for (const socket of mockSockets.values()) {
    for (const [subId, subscription] of socket.subscriptions) {
      if (
        (subscription.channelId === channelId ||
          subscription.channelId === GLOBAL_MOCK_SUBSCRIPTION) &&
        (!subscription.kinds || subscription.kinds.includes(event.kind))
      ) {
        sendWsText(socket.handler, ["EVENT", subId, event]);
      }
    }
  }
}

function hasMockLiveSubscription(channelId: string, kind?: number) {
  for (const socket of mockSockets.values()) {
    for (const subscription of socket.subscriptions.values()) {
      if (
        (subscription.channelId === channelId ||
          subscription.channelId === GLOBAL_MOCK_SUBSCRIPTION) &&
        (kind === undefined ||
          !subscription.kinds ||
          subscription.kinds.includes(kind))
      ) {
        return true;
      }
    }
  }

  return false;
}

function recordMockMessage(channelId: string, event: RelayEvent) {
  const history = getMockMessageStore(channelId);
  history.push(event);

  const channel = mockChannels.find((candidate) => candidate.id === channelId);
  if (!channel) {
    return;
  }

  channel.last_message_at = new Date(event.created_at * 1_000).toISOString();
  touchMockChannel(channel);
}

function emitMockChannelMessage(
  channelId: string,
  content: string,
  parentEventId?: string | null,
  pubkey?: string,
  kind?: number,
) {
  const eventKind = kind ?? 9;
  if (!parentEventId) {
    const event = createMockEvent(
      eventKind,
      content,
      [["h", channelId]],
      pubkey,
    );
    recordMockMessage(channelId, event);
    emitMockLiveEvent(channelId, event);
    return event;
  }

  const history = getMockMessageStore(channelId);
  const parentEvent =
    history.find((event) => event.id === parentEventId) ?? null;
  const parentThread = parentEvent
    ? getThreadReferenceFromTags(parentEvent.tags)
    : {
        parentEventId: null,
        rootEventId: null,
      };
  const rootEventId = parentThread.rootEventId ?? parentEventId;
  const authorPubkey = pubkey ?? DEFAULT_MOCK_IDENTITY.pubkey;
  const event = createMockEvent(
    eventKind,
    content,
    buildReplyMessageTags(
      channelId,
      authorPubkey,
      parentEventId,
      rootEventId,
      undefined,
    ),
    authorPubkey,
  );
  recordMockMessage(channelId, event);
  emitMockLiveEvent(channelId, event);
  return event;
}

function emitMockTypingIndicator(channelId: string, pubkey: string) {
  const event: RelayEvent = {
    id: crypto.randomUUID().replace(/-/g, ""),
    pubkey,
    created_at: Math.floor(Date.now() / 1000),
    kind: 20002,
    tags: [["h", channelId]],
    content: "",
    sig: "mocksig".repeat(20).slice(0, 128),
  };

  emitMockLiveEvent(channelId, event);
  return event;
}

function toRawForumPost(
  event: RelayEvent,
  channelId: string,
  threadSummary: RawThreadSummary | null,
): RawForumPost {
  return {
    event_id: event.id,
    pubkey: event.pubkey,
    content: event.content,
    kind: event.kind,
    created_at: event.created_at,
    channel_id: channelId,
    tags: event.tags,
    thread_summary: threadSummary,
    reactions: null,
  };
}

function toRawForumReply(event: RelayEvent, channelId: string): RawForumReply {
  const thread = getThreadReferenceFromTags(event.tags);

  return {
    event_id: event.id,
    pubkey: event.pubkey,
    content: event.content,
    kind: event.kind,
    created_at: event.created_at,
    channel_id: channelId,
    tags: event.tags,
    parent_event_id: thread.parentEventId,
    root_event_id: thread.rootEventId,
    depth:
      thread.rootEventId && thread.parentEventId !== thread.rootEventId ? 2 : 1,
    broadcast: false,
    reactions: null,
  };
}

async function handleGetForumPosts(args: {
  channelId: string;
  limit?: number | null;
  before?: number | null;
}): Promise<RawForumPostsResponse> {
  const events = getMockMessageStore(args.channelId);
  const posts = events
    .filter((event) => event.kind === 45001)
    .filter((event) => (args.before ? event.created_at < args.before : true))
    .sort((left, right) => right.created_at - left.created_at)
    .slice(0, args.limit ?? 50)
    .map((event) => {
      const replies = events.filter((candidate) => {
        if (candidate.kind !== 45003) {
          return false;
        }

        const thread = getThreadReferenceFromTags(candidate.tags);
        return (thread.rootEventId ?? thread.parentEventId) === event.id;
      });

      return toRawForumPost(event, args.channelId, {
        reply_count: replies.length,
        descendant_count: replies.length,
        last_reply_at:
          replies.length > 0 ? replies[replies.length - 1].created_at : null,
        participants: [...new Set(replies.map((reply) => reply.pubkey))],
      });
    });

  return {
    messages: posts,
    next_cursor: null,
  };
}

async function handleGetForumThread(args: {
  channelId: string;
  eventId: string;
}): Promise<RawForumThreadResponse> {
  const events = getMockMessageStore(args.channelId);
  const root = events.find(
    (event) => event.id === args.eventId && event.kind === 45001,
  );
  if (!root) {
    throw new Error(`Mock forum thread not found: ${args.eventId}`);
  }

  const replies = events
    .filter((event) => event.kind === 45003)
    .filter((event) => {
      const thread = getThreadReferenceFromTags(event.tags);
      return (thread.rootEventId ?? thread.parentEventId) === root.id;
    })
    .sort((left, right) => left.created_at - right.created_at)
    .map((event) => toRawForumReply(event, args.channelId));

  return {
    root: toRawForumPost(root, args.channelId, {
      reply_count: replies.length,
      descendant_count: replies.length,
      last_reply_at:
        replies.length > 0 ? replies[replies.length - 1].created_at : null,
      participants: [...new Set(replies.map((reply) => reply.pubkey))],
    }),
    replies,
    total_replies: replies.length,
    next_cursor: null,
  };
}

function getMockUserNotes(pubkey: string): RawUserNote[] {
  const now = Math.floor(Date.now() / 1000);

  if (pubkey === DEFAULT_MOCK_IDENTITY.pubkey) {
    return [
      {
        id: "mock-note-launch",
        pubkey,
        created_at: now - 20 * 60,
        content: "Shipped the new desktop sidebar polish today.",
      },
      {
        id: "mock-note-forum",
        pubkey,
        created_at: now - 3 * 60 * 60,
        content: "Forum threads feel like the right home for slower decisions.",
      },
    ];
  }

  if (pubkey === ALICE_PUBKEY) {
    return [
      {
        id: "mock-alice-note-release",
        pubkey,
        created_at: now - 45 * 60,
        content: "Release checklist is ready for async feedback.",
      },
      {
        id: "mock-alice-note-design",
        pubkey,
        created_at: now - 5 * 60 * 60,
        content: "Trying a lighter forum layout for longer-form notes.",
      },
    ];
  }

  return [];
}

async function handleGetUserNotes(
  args: {
    pubkey: string;
    limit?: number | null;
    before?: number | null;
    beforeId?: string | null;
  },
  config: E2eConfig | undefined,
): Promise<RawUserNotesResponse> {
  const identity = getIdentity(config);
  if (!identity) {
    const notes = getMockUserNotes(args.pubkey)
      .filter((note) => (args.before ? note.created_at < args.before : true))
      .sort((left, right) => right.created_at - left.created_at)
      .slice(0, args.limit ?? 50);

    return {
      notes,
      next_cursor: null,
    };
  }

  // Query kind:1 notes for the user
  const limit = args.limit ?? 50;
  const filter: Record<string, unknown> = {
    kinds: [1],
    authors: [args.pubkey],
    limit,
  };
  if (args.before !== undefined && args.before !== null) {
    filter.until = args.before;
  }
  const events = await relayQuery(config, [filter]);
  const notes = events.map((ev) => ({
    id: ev.id,
    pubkey: ev.pubkey,
    content: ev.content,
    created_at: ev.created_at,
    kind: ev.kind,
    tags: ev.tags,
    sig: ev.sig,
  }));
  return { notes, next_cursor: null };
}

function createMockEvent(
  kind: number,
  content: string,
  tags: string[][],
  pubkey = DEFAULT_MOCK_IDENTITY.pubkey,
  createdAt = Math.floor(Date.now() / 1000),
): RelayEvent {
  return {
    id: crypto.randomUUID().replace(/-/g, ""),
    pubkey,
    created_at: createdAt,
    kind,
    tags,
    content,
    sig: "mocksig".repeat(20).slice(0, 128),
  };
}

async function signWithIdentity(
  identity: TestIdentity,
  template: {
    kind: number;
    content: string;
    createdAt?: number;
    tags: string[][];
  },
) {
  const secretKey = hexToBytes(identity.privateKey);

  return finalizeEvent(
    {
      kind: template.kind,
      content: template.content,
      tags: template.tags,
      created_at: template.createdAt ?? Math.floor(Date.now() / 1000),
    },
    secretKey,
  );
}

async function assertOk(response: Response) {
  if (response.ok) {
    return;
  }

  const body = await response.text();
  throw new Error(body || `Request failed with ${response.status}`);
}

function getRelayIdentity(config: E2eConfig | undefined): TestIdentity {
  const identity = getIdentity(config);
  if (!identity) {
    throw new Error("Relay identity required.");
  }

  return identity;
}

async function relayJsonRequest<T>(
  config: E2eConfig | undefined,
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const identity = getRelayIdentity(config);
  const headers = new Headers(init.headers);

  headers.set("X-Pubkey", identity.pubkey);
  if (init.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  const response = await fetch(`${getRelayHttpUrl(config)}${path}`, {
    ...init,
    headers,
  });
  await assertOk(response);
  return response.json() as Promise<T>;
}

/**
 * Query the relay via POST /query (pure Nostr HTTP bridge).
 * Returns an array of raw Nostr events matching the filters.
 */
async function relayQuery(
  config: E2eConfig | undefined,
  filters: Array<Record<string, unknown>>,
): Promise<RelayEvent[]> {
  const identity = getRelayIdentity(config);
  const response = await fetch(`${getRelayHttpUrl(config)}/query`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Pubkey": identity.pubkey,
    },
    body: JSON.stringify(filters),
  });
  await assertOk(response);
  return response.json() as Promise<RelayEvent[]>;
}

async function submitSignedEvent(
  config: E2eConfig | undefined,
  template: { kind: number; content: string; tags: string[][] },
): Promise<{ event_id: string; accepted: boolean; message: string }> {
  const identity = getRelayIdentity(config);
  const signed = await signWithIdentity(identity, template);
  return relayJsonRequest(config, "/events", {
    method: "POST",
    body: JSON.stringify(signed),
  });
}

async function handleGetChannels(config: E2eConfig | undefined) {
  const identity = getIdentity(config);
  if (!identity) {
    return listMockChannels(config);
  }

  // Pure Nostr: query kind:39002 (membership) for our pubkey, extract channel
  // UUIDs from d-tags, then query kind:39000 (metadata) for those channels.
  const memberEvents = await relayQuery(config, [
    { kinds: [39002], "#p": [identity.pubkey], limit: 1000 },
  ]);

  const channelIds = [
    ...new Set(
      memberEvents.flatMap((ev) =>
        (ev.tags ?? [])
          .filter((t: string[]) => t[0] === "d")
          .map((t: string[]) => t[1]),
      ),
    ),
  ];

  // Also fetch ALL open channel metadata (for channel browser — shows joinable channels)
  const allMetaEvents = await relayQuery(config, [
    { kinds: [39000], limit: 200 },
  ]);

  // Merge: use all metadata events, mark membership
  const memberSet = new Set(channelIds);
  const metaEvents = allMetaEvents;

  // Convert kind:39000 events to the RawChannel shape the frontend expects.
  return metaEvents.map((ev) => {
    const tags = (ev.tags ?? []) as string[][];
    const getTag = (name: string) =>
      tags.find((t) => t[0] === name)?.[1] ?? null;
    const channelId = getTag("d") ?? "";
    const channelType = getTag("t") ?? "stream";
    const isPrivate = tags.some((t) => t[0] === "private");
    const isArchived = tags.some((t) => t[0] === "archived" && t[1] === "true");

    // Get participant pubkeys from the membership event for this channel
    const memberEvent = memberEvents.find((me) =>
      (me.tags ?? []).some((t: string[]) => t[0] === "d" && t[1] === channelId),
    );
    const pTags = memberEvent
      ? ((memberEvent.tags ?? []) as string[][])
          .filter((t) => t[0] === "p")
          .map((t) => t[1])
      : [];

    return {
      id: channelId,
      name: getTag("name") ?? "",
      description: getTag("about") ?? "",
      channel_type: channelType as "stream" | "forum" | "dm",
      visibility: (isPrivate ? "private" : "open") as "open" | "private",
      topic: getTag("topic") ?? null,
      purpose: getTag("purpose") ?? null,
      member_count: pTags.length,
      last_message_at: null,
      archived_at: isArchived ? new Date().toISOString() : null,
      participants: pTags,
      participant_pubkeys: pTags,
      ttl_seconds: getTag("ttl") ? Number(getTag("ttl")) : null,
      ttl_deadline: getTag("ttl_deadline") ?? null,
      is_member: memberSet.has(channelId),
    };
  });
}

async function handleGetProfile(config: E2eConfig | undefined) {
  const identity = getIdentity(config);
  if (!identity) {
    const profileReadDelayMs = config?.mock?.profileReadDelayMs ?? 0;
    if (profileReadDelayMs > 0) {
      await new Promise<void>((resolve) => {
        window.setTimeout(resolve, profileReadDelayMs);
      });
    }

    const profileReadError = config?.mock?.profileReadError;
    if (profileReadError) {
      throw new Error(profileReadError);
    }

    return cloneProfile(ensureMockProfile(config));
  }

  // Pure Nostr: query kind:0 (profile metadata) for our pubkey.
  const events = await relayQuery(config, [
    { kinds: [0], authors: [identity.pubkey], limit: 1 },
  ]);
  if (events.length === 0) {
    return {
      pubkey: identity.pubkey,
      display_name: null,
      about: null,
      avatar_url: null,
      nip05: null,
    };
  }
  const content = JSON.parse(events[0].content ?? "{}");
  return {
    pubkey: identity.pubkey,
    display_name: content.display_name ?? content.name ?? null,
    about: content.about ?? null,
    avatar_url: content.picture ?? null,
    nip05: content.nip05 ?? null,
  };
}

async function handleUpdateProfile(
  args: {
    displayName?: string;
    avatarUrl?: string;
    about?: string;
    nip05Handle?: string;
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const profileUpdateError = config?.mock?.profileUpdateError;
    if (profileUpdateError) {
      if (config?.mock) {
        config.mock.profileUpdateError = undefined;
      }
      throw new Error(profileUpdateError);
    }

    const profile = ensureMockProfile(config);
    const nextDisplayName = args.displayName?.trim();
    const nextAvatarUrl = args.avatarUrl?.trim();
    const nextAbout = args.about?.trim();
    const nextNip05Handle = args.nip05Handle?.trim();

    if (nextDisplayName && nextDisplayName !== profile.display_name) {
      profile.display_name = nextDisplayName;
      applyMockDisplayName(profile.pubkey, nextDisplayName);
    }
    if (nextAvatarUrl && nextAvatarUrl !== profile.avatar_url) {
      profile.avatar_url = nextAvatarUrl;
    }
    if (nextAbout && nextAbout !== profile.about) {
      profile.about = nextAbout;
    }
    if (
      typeof nextNip05Handle === "string" &&
      nextNip05Handle !== profile.nip05_handle
    ) {
      profile.nip05_handle =
        nextNip05Handle.length > 0 ? nextNip05Handle : null;
    }

    return cloneProfile(profile);
  }

  // Read-merge-write: fetch current profile, merge, sign kind:0.
  const currentEvents = await relayQuery(config, [
    { kinds: [0], authors: [identity.pubkey], limit: 1 },
  ]);
  const currentContent = currentEvents[0]
    ? JSON.parse(currentEvents[0].content ?? "{}")
    : {};
  const profileContent = JSON.stringify({
    display_name: args.displayName ?? currentContent.display_name ?? undefined,
    name: currentContent.display_name ?? undefined,
    picture: args.avatarUrl ?? currentContent.picture ?? undefined,
    about: args.about ?? currentContent.about ?? undefined,
    nip05: args.nip05Handle ?? currentContent.nip05 ?? undefined,
  });
  await submitSignedEvent(config, {
    kind: 0,
    content: profileContent,
    tags: [],
  });

  // Return the updated profile in RawProfile shape
  const updated = JSON.parse(profileContent);
  return {
    pubkey: identity.pubkey,
    display_name: updated.display_name ?? null,
    about: updated.about ?? null,
    avatar_url: updated.picture ?? null,
    nip05: updated.nip05 ?? null,
  };
}

async function handleGetUserProfile(
  args: {
    pubkey?: string;
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const pubkey = (args.pubkey ?? getMockMemberPubkey(config)).toLowerCase();
    const profile = getMockProfileByPubkey(pubkey);
    if (!profile) {
      throw new Error(`User ${pubkey} not found.`);
    }

    return cloneProfile(profile);
  }

  const targetPubkey = args.pubkey ?? identity.pubkey;
  const events = await relayQuery(config, [
    { kinds: [0], authors: [targetPubkey], limit: 1 },
  ]);
  if (events.length === 0) {
    return {
      pubkey: targetPubkey,
      display_name: null,
      about: null,
      avatar_url: null,
      nip05: null,
    };
  }
  const content = JSON.parse(events[0].content ?? "{}");
  return {
    pubkey: targetPubkey,
    display_name: content.display_name ?? content.name ?? null,
    about: content.about ?? null,
    avatar_url: content.picture ?? null,
    nip05: content.nip05 ?? null,
  };
}

async function handleGetUsersBatch(
  args: {
    pubkeys: string[];
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const profiles: RawUsersBatchResponse["profiles"] = {};
    const missing: string[] = [];

    for (const pubkey of args.pubkeys) {
      const normalizedPubkey = pubkey.toLowerCase();
      const profile = getMockProfileByPubkey(normalizedPubkey);

      if (!profile) {
        missing.push(pubkey);
        continue;
      }

      profiles[normalizedPubkey] = {
        display_name: profile.display_name,
        avatar_url: profile.avatar_url,
        nip05_handle: profile.nip05_handle,
      };
    }

    return {
      profiles,
      missing,
    };
  }

  const events = await relayQuery(config, [
    { kinds: [0], authors: args.pubkeys, limit: args.pubkeys.length },
  ]);
  const profiles: RawUsersBatchResponse["profiles"] = {};
  const found = new Set<string>();
  for (const ev of events) {
    const pk = ev.pubkey?.toLowerCase() ?? "";
    found.add(pk);
    const content = JSON.parse(ev.content ?? "{}");
    profiles[pk] = {
      display_name: content.display_name ?? content.name ?? null,
      avatar_url: content.picture ?? null,
      nip05_handle: content.nip05 ?? null,
    };
  }
  const missing = args.pubkeys.filter((p) => !found.has(p.toLowerCase()));
  return { profiles, missing };
}

async function handleSearchUsers(
  args: {
    query: string;
    limit?: number;
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const normalizedQuery = args.query.trim().toLowerCase();
    if (normalizedQuery.length === 0) {
      return { users: [] } satisfies RawSearchUsersResponse;
    }

    const results = listMockProfiles()
      .filter((profile) => {
        const displayName = profile.display_name?.toLowerCase() ?? "";
        const nip05Handle = profile.nip05_handle?.toLowerCase() ?? "";
        const pubkey = profile.pubkey.toLowerCase();
        return (
          displayName.includes(normalizedQuery) ||
          nip05Handle.includes(normalizedQuery) ||
          pubkey.includes(normalizedQuery)
        );
      })
      .sort((left, right) => {
        const leftName = left.display_name ?? left.nip05_handle ?? left.pubkey;
        const rightName =
          right.display_name ?? right.nip05_handle ?? right.pubkey;
        return leftName.localeCompare(rightName);
      })
      .slice(0, args.limit ?? 8)
      .map((profile) => ({
        pubkey: profile.pubkey,
        display_name: profile.display_name,
        avatar_url: profile.avatar_url,
        nip05_handle: profile.nip05_handle,
      }));

    return {
      users: results,
    } satisfies RawSearchUsersResponse;
  }

  // NIP-50 search on kind:0 profiles
  const limit = args.limit ?? 8;
  const events = await relayQuery(config, [
    { kinds: [0], search: args.query, limit },
  ]);
  const users = events.map((ev) => {
    const content = JSON.parse(ev.content ?? "{}");
    return {
      pubkey: ev.pubkey ?? "",
      display_name: content.display_name ?? content.name ?? null,
      avatar_url: content.picture ?? null,
      nip05_handle: content.nip05 ?? null,
    };
  });
  return { users };
}

async function handleGetPresence(
  args: {
    pubkeys: string[];
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    return Object.fromEntries(
      args.pubkeys.map((pubkey) => [
        pubkey.toLowerCase(),
        getMockPresenceStatus(pubkey),
      ]),
    ) satisfies RawPresenceLookup;
  }

  if (args.pubkeys.length === 0) {
    return {} satisfies RawPresenceLookup;
  }

  // Presence is ephemeral (kind:20001) — query via bridge which synthesizes from Redis.
  const events = await relayQuery(config, [
    { kinds: [20001], authors: args.pubkeys, limit: args.pubkeys.length },
  ]);
  const result: RawPresenceLookup = {};
  for (const ev of events) {
    // Synthesized presence events have ["p", subject_pubkey] tag
    const pTag = ((ev.tags ?? []) as string[][]).find((t) => t[0] === "p");
    const pk = pTag?.[1] ?? ev.pubkey ?? "";
    result[pk.toLowerCase()] = (ev.content ?? "offline") as PresenceStatus;
  }
  // Fill missing pubkeys with "offline"
  for (const pk of args.pubkeys) {
    if (!result[pk.toLowerCase()]) {
      result[pk.toLowerCase()] = "offline";
    }
  }
  return result;
}

async function handleSetPresence(
  args: {
    status: PresenceStatus;
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    setMockPresenceStatus(getMockMemberPubkey(config), args.status);

    return {
      status: args.status,
      ttl_seconds: args.status === "offline" ? 0 : MOCK_PRESENCE_TTL_SECONDS,
    } satisfies RawSetPresenceResponse;
  }

  // Presence is ephemeral kind:20001 — submit via POST /events.
  // Note: the relay may reject this with "kind 20001 is only accepted via WebSocket"
  // in which case we just return the expected shape (presence is best-effort in e2e).
  try {
    await submitSignedEvent(config, {
      kind: 20001,
      content: args.status,
      tags: [],
    });
  } catch {
    // Expected: ephemeral events may be WS-only
  }
  return {
    status: args.status,
    ttl_seconds: args.status === "offline" ? 0 : 90,
  };
}

async function handleCreateChannel(
  args: {
    name: string;
    channelType: "stream" | "forum";
    visibility: "open" | "private";
    description?: string;
    ttlSeconds?: number;
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  const ttlDeadline =
    typeof args.ttlSeconds === "number"
      ? new Date(Date.now() + args.ttlSeconds * 1_000).toISOString()
      : null;
  if (!identity) {
    const owner = createCurrentMember(config, "owner");
    const channel = createMockChannel({
      id: crypto.randomUUID(),
      name: args.name,
      channel_type: args.channelType,
      visibility: args.visibility,
      description: args.description ?? "",
      topic: null,
      purpose: null,
      last_message_at: null,
      archived_at: null,
      created_by: owner.pubkey,
      topic_set_by: null,
      topic_set_at: null,
      purpose_set_by: null,
      purpose_set_at: null,
      ttl_seconds: args.ttlSeconds ?? null,
      ttl_deadline: ttlDeadline,
      topic_required: false,
      max_members: null,
      nip29_group_id: null,
      created_minutes_ago: 0,
      updated_minutes_ago: 0,
      members: [owner],
    });
    mockChannels.push(channel);
    return toRawChannel(channel, config);
  }

  const channelId = crypto.randomUUID();
  const tags: string[][] = [
    ["h", channelId],
    ["name", args.name],
    ["channel_type", args.channelType],
    ["visibility", args.visibility],
  ];
  if (args.description) {
    tags.push(["about", args.description]);
  }
  if (typeof args.ttlSeconds === "number") {
    tags.push(["ttl", String(args.ttlSeconds)]);
  }
  await submitSignedEvent(config, { kind: 9007, content: "", tags });

  // Fetch the created channel via pure Nostr query.
  // The relay emits kind:39000 as a side effect of kind:9007.
  const metaEvents = await relayQuery(config, [
    { kinds: [39000], "#d": [channelId], limit: 1 },
  ]);
  const ev = metaEvents[0];
  if (!ev) {
    throw new Error(`Channel "${args.name}" not found after creation`);
  }
  const evTags = (ev.tags ?? []) as string[][];
  const getTag = (name: string) =>
    evTags.find((t) => t[0] === name)?.[1] ?? null;
  return {
    id: channelId,
    name: getTag("name") ?? args.name,
    description: getTag("about") ?? args.description ?? null,
    channel_type: args.channelType,
    visibility: args.visibility,
    topic: null,
    purpose: null,
    role: "owner",
    archived_at: null,
    ttl_seconds: args.ttlSeconds ?? null,
    ttl_deadline: ttlDeadline,
    created_at: ev.created_at
      ? new Date(ev.created_at * 1000).toISOString()
      : new Date().toISOString(),
  };
}

async function handleOpenDm(
  args: {
    pubkeys: string[];
  },
  config: E2eConfig | undefined,
) {
  const normalizedPubkeys = normalizeParticipantPubkeys(args.pubkeys);
  if (normalizedPubkeys.length === 0) {
    throw new Error("Select at least one person to start a DM.");
  }

  const currentPubkey = getMockMemberPubkey(config).toLowerCase();
  const participantPubkeys = normalizeParticipantPubkeys([
    currentPubkey,
    ...normalizedPubkeys.filter((pubkey) => pubkey !== currentPubkey),
  ]);
  const existingChannel = findMockDmByParticipantPubkeys(participantPubkeys);
  if (existingChannel) {
    return toRawChannel(existingChannel, config);
  }

  const identity = getIdentity(config);
  if (!identity) {
    const members = participantPubkeys.map((pubkey) =>
      createMockMember(pubkey, "member", 0),
    );
    const channel = createMockChannel({
      id: crypto.randomUUID(),
      name:
        participantPubkeys.length === 2
          ? "DM"
          : `Group DM (${participantPubkeys.length})`,
      channel_type: "dm",
      visibility: "private",
      description: "Direct message conversation",
      topic: null,
      purpose: null,
      last_message_at: null,
      archived_at: null,
      created_by: getMockMemberPubkey(config),
      topic_set_by: null,
      topic_set_at: null,
      purpose_set_by: null,
      purpose_set_at: null,
      topic_required: false,
      max_members: participantPubkeys.length,
      nip29_group_id: null,
      created_minutes_ago: 0,
      updated_minutes_ago: 0,
      members,
    });
    syncMockChannel(channel);
    mockChannels.push(channel);
    return toRawChannel(channel, config);
  }

  // Submit kind:41010 (DM open) with p-tags for participants
  const tags = normalizedPubkeys.map((pk) => ["p", pk]);
  const result = await submitSignedEvent(config, {
    kind: 41010,
    content: "",
    tags,
  });
  // Parse channel_id from response message
  const respJson = JSON.parse(result.message.replace("response:", "") || "{}");
  const channelId = respJson.channel_id ?? "";

  // Fetch channel metadata
  const metaEvents = await relayQuery(config, [
    { kinds: [39000], "#d": [channelId], limit: 1 },
  ]);
  const ev = metaEvents[0];
  const evTags = (ev?.tags ?? []) as string[][];
  const getTag = (name: string) =>
    evTags.find((t) => t[0] === name)?.[1] ?? null;
  return {
    id: channelId,
    name: getTag("name") ?? "DM",
    description: null,
    channel_type: "dm",
    visibility: "private",
    topic: null,
    purpose: null,
    role: "member",
    archived_at: null,
    ttl_seconds: null,
    ttl_deadline: null,
    created_at: ev?.created_at
      ? new Date(ev.created_at * 1000).toISOString()
      : new Date().toISOString(),
  };
}

async function handleHideDm(
  args: { channelId: string },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const index = mockChannels.findIndex(
      (channel) => channel.id === args.channelId,
    );
    if (index === -1) {
      throw new Error(`DM ${args.channelId} not found.`);
    }
    // Remove from mock list (simulates hiding from sidebar).
    mockChannels.splice(index, 1);
    return;
  }

  // Submit kind:41012 (DM hide) with h-tag
  await submitSignedEvent(config, {
    kind: 41012,
    content: "",
    tags: [["h", args.channelId]],
  });
}

async function handleGetChannelDetails(
  args: { channelId: string },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    return toRawChannelDetail(getMockChannel(args.channelId), config);
  }

  const metaEvents = await relayQuery(config, [
    { kinds: [39000], "#d": [args.channelId], limit: 1 },
  ]);
  const ev = metaEvents[0];
  const evTags = (ev?.tags ?? []) as string[][];
  const getTag = (name: string) =>
    evTags.find((t) => t[0] === name)?.[1] ?? null;

  // Get members for member_count
  const memberEvents = await relayQuery(config, [
    { kinds: [39002], "#d": [args.channelId], limit: 1 },
  ]);
  const memberTags = ((memberEvents[0]?.tags ?? []) as string[][]).filter(
    (t) => t[0] === "p",
  );

  return {
    id: args.channelId,
    name: getTag("name") ?? "",
    description: getTag("about") ?? null,
    channel_type: getTag("t") ?? "stream",
    visibility: evTags.some((t) => t[0] === "private") ? "private" : "open",
    topic: getTag("topic") ?? null,
    purpose: getTag("purpose") ?? null,
    member_count: memberTags.length,
    role: "member",
    archived_at: evTags.some((t) => t[0] === "archived" && t[1] === "true")
      ? new Date().toISOString()
      : null,
    ttl_seconds: getTag("ttl") ? Number(getTag("ttl")) : null,
    ttl_deadline: getTag("ttl_deadline") ?? null,
    created_at: ev?.created_at
      ? new Date(ev.created_at * 1000).toISOString()
      : new Date().toISOString(),
  };
}

async function handleGetChannelMembers(
  args: { channelId: string },
  config: E2eConfig | undefined,
): Promise<RawChannelMembersResponse> {
  const identity = getIdentity(config);
  if (!identity) {
    const channel = getMockChannel(args.channelId);
    return {
      members: cloneMembers(channel.members),
      next_cursor: null,
    };
  }

  const memberEvents = await relayQuery(config, [
    { kinds: [39002], "#d": [args.channelId], limit: 1 },
  ]);
  const memberTags = ((memberEvents[0]?.tags ?? []) as string[][]).filter(
    (t) => t[0] === "p",
  );
  const members = memberTags.map((t) => ({
    pubkey: t[1],
    role: (t[3] ?? t[2] ?? "member") as
      | "owner"
      | "admin"
      | "member"
      | "guest"
      | "bot",
    display_name: null,
    avatar_url: null,
    joined_at: new Date().toISOString(),
  }));
  return { members, next_cursor: null };
}

async function handleUpdateChannel(
  args: {
    channelId: string;
    name?: string;
    description?: string;
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const channel = getMockChannel(args.channelId);
    if (args.name !== undefined) {
      channel.name = args.name;
    }
    if (args.description !== undefined) {
      channel.description = args.description;
    }
    touchMockChannel(channel);
    return toRawChannelDetail(channel, config);
  }

  const tags: string[][] = [["h", args.channelId]];
  if (args.name !== undefined) {
    tags.push(["name", args.name]);
  }
  if (args.description !== undefined) {
    tags.push(["about", args.description]);
  }
  await submitSignedEvent(config, { kind: 9002, content: "", tags });

  // Re-fetch updated metadata
  const metaEvents = await relayQuery(config, [
    { kinds: [39000], "#d": [args.channelId], limit: 1 },
  ]);
  const ev = metaEvents[0];
  const evTags = (ev?.tags ?? []) as string[][];
  const getTag = (name: string) =>
    evTags.find((t) => t[0] === name)?.[1] ?? null;
  return {
    id: args.channelId,
    name: getTag("name") ?? "",
    description: getTag("about") ?? null,
    channel_type: getTag("t") ?? "stream",
    visibility: evTags.some((t) => t[0] === "private") ? "private" : "open",
    topic: getTag("topic") ?? null,
    purpose: getTag("purpose") ?? null,
    member_count: 0,
    role: "owner",
    archived_at: null,
    ttl_seconds: null,
    ttl_deadline: null,
    created_at: ev?.created_at
      ? new Date(ev.created_at * 1000).toISOString()
      : new Date().toISOString(),
  };
}

async function handleSetChannelTopic(
  args: {
    channelId: string;
    topic: string;
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const channel = getMockChannel(args.channelId);
    const nextTopic = args.topic.trim();

    channel.topic = nextTopic.length > 0 ? nextTopic : null;
    channel.topic_set_by = getMockMemberPubkey(config);
    channel.topic_set_at = new Date().toISOString();
    touchMockChannel(channel);
    return;
  }

  await submitSignedEvent(config, {
    kind: 9002,
    content: "",
    tags: [
      ["h", args.channelId],
      ["topic", args.topic],
    ],
  });
}

async function handleSetChannelPurpose(
  args: {
    channelId: string;
    purpose: string;
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const channel = getMockChannel(args.channelId);
    const nextPurpose = args.purpose.trim();

    channel.purpose = nextPurpose.length > 0 ? nextPurpose : null;
    channel.purpose_set_by = getMockMemberPubkey(config);
    channel.purpose_set_at = new Date().toISOString();
    touchMockChannel(channel);
    return;
  }

  await submitSignedEvent(config, {
    kind: 9002,
    content: "",
    tags: [
      ["h", args.channelId],
      ["purpose", args.purpose],
    ],
  });
}

async function handleArchiveChannel(
  args: { channelId: string },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const channel = getMockChannel(args.channelId);
    channel.archived_at = new Date().toISOString();
    touchMockChannel(channel);
    return;
  }

  await submitSignedEvent(config, {
    kind: 9002,
    content: "",
    tags: [
      ["h", args.channelId],
      ["archived", "true"],
    ],
  });
}

async function handleUnarchiveChannel(
  args: { channelId: string },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const channel = getMockChannel(args.channelId);
    channel.archived_at = null;
    touchMockChannel(channel);
    return;
  }

  await submitSignedEvent(config, {
    kind: 9002,
    content: "",
    tags: [
      ["h", args.channelId],
      ["archived", "false"],
    ],
  });
}

async function handleDeleteChannel(
  args: { channelId: string },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const index = mockChannels.findIndex(
      (channel) => channel.id === args.channelId,
    );
    if (index === -1) {
      throw new Error(`Channel ${args.channelId} not found.`);
    }

    mockChannels.splice(index, 1);
    mockMessages.delete(args.channelId);
    return;
  }

  await submitSignedEvent(config, {
    kind: 9008,
    content: "",
    tags: [["h", args.channelId]],
  });
}

async function handleAddChannelMembers(
  args: {
    channelId: string;
    pubkeys: string[];
    role?: RawChannelMember["role"];
  },
  config: E2eConfig | undefined,
): Promise<RawAddChannelMembersResponse> {
  const identity = getIdentity(config);
  if (!identity) {
    const channel = getMockChannel(args.channelId);
    const added: string[] = [];
    const errors: RawAddChannelMembersResponse["errors"] = [];

    for (const pubkey of args.pubkeys) {
      if (channel.members.some((member) => member.pubkey === pubkey)) {
        errors.push({
          pubkey,
          error: "Already a member.",
        });
        continue;
      }

      channel.members.push({
        pubkey,
        role: args.role ?? "member",
        joined_at: new Date().toISOString(),
        display_name: mockDisplayNames.get(pubkey) ?? null,
      });
      added.push(pubkey);
    }

    syncMockChannel(channel);
    touchMockChannel(channel);
    syncMockRelayAgentsFromManagedAgents();
    return {
      added,
      errors,
    };
  }

  const added: string[] = [];
  const errors: RawAddChannelMembersResponse["errors"] = [];
  for (const pubkey of args.pubkeys) {
    try {
      const tags: string[][] = [
        ["h", args.channelId],
        ["p", pubkey],
      ];
      if (args.role) {
        tags.push(["role", args.role]);
      }
      await submitSignedEvent(config, { kind: 9000, content: "", tags });
      added.push(pubkey);
    } catch (e) {
      errors.push({ pubkey, error: String(e) });
    }
  }
  return { added, errors };
}

async function handleRemoveChannelMember(
  args: {
    channelId: string;
    pubkey: string;
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const channel = getMockChannel(args.channelId);
    channel.members = channel.members.filter(
      (member) => member.pubkey !== args.pubkey,
    );
    syncMockChannel(channel);
    touchMockChannel(channel);
    syncMockRelayAgentsFromManagedAgents();
    return;
  }

  await submitSignedEvent(config, {
    kind: 9001,
    content: "",
    tags: [
      ["h", args.channelId],
      ["p", args.pubkey],
    ],
  });
}

async function handleJoinChannel(
  args: {
    channelId: string;
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const channel = getMockChannel(args.channelId);
    const currentPubkey = getMockMemberPubkey(config);

    if (channel.members.some((member) => member.pubkey === currentPubkey)) {
      return;
    }

    channel.members.push(createCurrentMember(config, "member"));
    syncMockChannel(channel);
    touchMockChannel(channel);
    return;
  }

  await submitSignedEvent(config, {
    kind: 9021,
    content: "",
    tags: [["h", args.channelId]],
  });
}

async function handleLeaveChannel(
  args: {
    channelId: string;
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const channel = getMockChannel(args.channelId);
    const currentPubkey = getMockMemberPubkey(config);

    channel.members = channel.members.filter(
      (member) => member.pubkey !== currentPubkey,
    );
    syncMockChannel(channel);
    touchMockChannel(channel);
    return;
  }

  await submitSignedEvent(config, {
    kind: 9022,
    content: "",
    tags: [["h", args.channelId]],
  });
}

async function handleGetFeed(
  args: {
    since?: number;
    limit?: number;
    types?: string;
  },
  config: E2eConfig | undefined,
): Promise<RawHomeFeedResponse> {
  const identity = getIdentity(config);
  if (!identity) {
    const now = Math.floor(Date.now() / 1000);
    const limit = args.limit ?? 50;
    const wantedTypes =
      args.types
        ?.split(",")
        .map((value) => value.trim())
        .filter((value) => value.length > 0) ?? [];
    const includeType = (type: string) =>
      wantedTypes.length === 0 || wantedTypes.includes(type);

    const currentPubkey = getMockMemberPubkey(config).toLowerCase();
    const defaultFeed: RawHomeFeedResponse["feed"] =
      currentPubkey === ALICE_PUBKEY
        ? {
            mentions: [
              {
                id: "mock-feed-alice-mention",
                kind: 9,
                pubkey: BOB_PUBKEY,
                content: "Alice, can you sanity-check the new design mocks?",
                created_at: now - 90,
                channel_id: "b5e2f8a1-3c44-5912-9e67-4a8d1f2b3c4e",
                channel_name: "design",
                tags: [
                  ["e", "b5e2f8a1-3c44-5912-9e67-4a8d1f2b3c4e"],
                  ["p", ALICE_PUBKEY],
                ],
                category: "mention" as const,
              },
            ],
            needs_action: [
              {
                id: "mock-feed-alice-reminder",
                kind: 40007,
                pubkey:
                  "0000000000000000000000000000000000000000000000000000000000000000",
                content: "Reminder: post the engineering launch note.",
                created_at: now - 15 * 60,
                channel_id: "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9",
                channel_name: "engineering",
                tags: [
                  ["e", "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9"],
                  ["p", ALICE_PUBKEY],
                ],
                category: "needs_action" as const,
              },
            ],
            activity: [
              {
                id: "mock-feed-alice-self-activity",
                kind: 9,
                pubkey: ALICE_PUBKEY,
                content: "I posted the latest design review summary.",
                created_at: now - 25 * 60,
                channel_id: "b5e2f8a1-3c44-5912-9e67-4a8d1f2b3c4e",
                channel_name: "design",
                tags: [["e", "b5e2f8a1-3c44-5912-9e67-4a8d1f2b3c4e"]],
                category: "activity" as const,
              },
              {
                id: "mock-feed-alice-activity",
                kind: 9,
                pubkey: BOB_PUBKEY,
                content: "Engineering signed off on the desktop build.",
                created_at: now - 42 * 60,
                channel_id: "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9",
                channel_name: "engineering",
                tags: [["e", "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9"]],
                category: "activity" as const,
              },
            ],
            agent_activity: [
              {
                id: "mock-feed-alice-agent",
                kind: 43003,
                pubkey:
                  "db0b028cd36f4d3e36c8300cce87252c1f7fc9495ffecc53f393fcac341ffd36",
                content: "Agent progress: design review summary complete.",
                created_at: now - 2 * 60 * 60,
                channel_id: "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9",
                channel_name: "engineering",
                tags: [["e", "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9"]],
                category: "agent_activity" as const,
              },
            ],
          }
        : currentPubkey === DEFAULT_REAL_IDENTITY.pubkey.toLowerCase()
          ? {
              mentions: [
                {
                  id: "mock-feed-tyler-mention",
                  kind: 9,
                  pubkey: ALICE_PUBKEY,
                  content: "Tyler, can you review the DM onboarding copy?",
                  created_at: now - 90,
                  channel_id: "f48efb06-0c93-5025-aac9-2e646bb6bfa8",
                  channel_name: "alice-tyler",
                  tags: [
                    ["e", "f48efb06-0c93-5025-aac9-2e646bb6bfa8"],
                    ["p", DEFAULT_REAL_IDENTITY.pubkey],
                  ],
                  category: "mention" as const,
                },
              ],
              needs_action: [
                {
                  id: "mock-feed-tyler-reminder",
                  kind: 40007,
                  pubkey:
                    "0000000000000000000000000000000000000000000000000000000000000000",
                  content: "Reminder: answer Bob in the launch DM thread.",
                  created_at: now - 15 * 60,
                  channel_id: "7eb9f239-9393-50b0-bd76-d85eef0511c7",
                  channel_name: "bob-tyler",
                  tags: [
                    ["e", "7eb9f239-9393-50b0-bd76-d85eef0511c7"],
                    ["p", DEFAULT_REAL_IDENTITY.pubkey],
                  ],
                  category: "needs_action" as const,
                },
              ],
              activity: [
                {
                  id: "mock-feed-tyler-self-activity",
                  kind: 9,
                  pubkey: DEFAULT_REAL_IDENTITY.pubkey,
                  content: "I sent the follow-up in the Alice DM.",
                  created_at: now - 25 * 60,
                  channel_id: "f48efb06-0c93-5025-aac9-2e646bb6bfa8",
                  channel_name: "alice-tyler",
                  tags: [["e", "f48efb06-0c93-5025-aac9-2e646bb6bfa8"]],
                  category: "activity" as const,
                },
              ],
              agent_activity: [
                {
                  id: "mock-feed-tyler-agent",
                  kind: 43003,
                  pubkey:
                    "db0b028cd36f4d3e36c8300cce87252c1f7fc9495ffecc53f393fcac341ffd36",
                  content: "Agent progress: DM summary complete.",
                  created_at: now - 2 * 60 * 60,
                  channel_id: "f48efb06-0c93-5025-aac9-2e646bb6bfa8",
                  channel_name: "alice-tyler",
                  tags: [["e", "f48efb06-0c93-5025-aac9-2e646bb6bfa8"]],
                  category: "agent_activity" as const,
                },
              ],
            }
          : {
              mentions: [
                {
                  id: "mock-feed-mention",
                  kind: 9,
                  pubkey: ALICE_PUBKEY,
                  content: "Please review the release checklist.",
                  created_at: now - 90,
                  channel_id: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
                  channel_name: "general",
                  tags: [
                    ["e", "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50"],
                    ["p", currentPubkey],
                  ],
                  category: "mention" as const,
                },
              ],
              needs_action: [
                {
                  id: "mock-feed-reminder",
                  kind: 40007,
                  pubkey:
                    "0000000000000000000000000000000000000000000000000000000000000000",
                  content: "Reminder: update the launch plan before lunch.",
                  created_at: now - 15 * 60,
                  channel_id: "94a444a4-c0a3-5966-ab05-530c6ddc2301",
                  channel_name: "agents",
                  tags: [
                    ["e", "94a444a4-c0a3-5966-ab05-530c6ddc2301"],
                    ["p", currentPubkey],
                  ],
                  category: "needs_action" as const,
                },
              ],
              activity: [
                {
                  id: "mock-feed-self-activity",
                  kind: 9,
                  pubkey: currentPubkey,
                  content: "I posted a note about the launch checklist.",
                  created_at: now - 25 * 60,
                  channel_id: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
                  channel_name: "general",
                  tags: [["e", "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50"]],
                  category: "activity" as const,
                },
                {
                  id: "mock-feed-activity",
                  kind: 9,
                  pubkey: BOB_PUBKEY,
                  content: "Engineering shipped the desktop build.",
                  created_at: now - 42 * 60,
                  channel_id: "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9",
                  channel_name: "engineering",
                  tags: [["e", "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9"]],
                  category: "activity" as const,
                },
              ],
              agent_activity: [
                {
                  id: "mock-feed-agent",
                  kind: 43003,
                  pubkey:
                    "db0b028cd36f4d3e36c8300cce87252c1f7fc9495ffecc53f393fcac341ffd36",
                  content: "Agent progress: channel index complete.",
                  created_at: now - 2 * 60 * 60,
                  channel_id: "94a444a4-c0a3-5966-ab05-530c6ddc2301",
                  channel_name: "agents",
                  tags: [["e", "94a444a4-c0a3-5966-ab05-530c6ddc2301"]],
                  category: "agent_activity" as const,
                },
              ],
            };

    const mergeFeedCategory = (
      category: keyof RawHomeFeedResponse["feed"],
    ): RawFeedItem[] =>
      includeType(category)
        ? [...mockFeedOverrides[category], ...defaultFeed[category]]
            .sort((left, right) => right.created_at - left.created_at)
            .slice(0, limit)
        : [];

    const mentions = mergeFeedCategory("mentions");
    const needsAction = mergeFeedCategory("needs_action");
    const activity = mergeFeedCategory("activity");
    const agentActivity = mergeFeedCategory("agent_activity");

    return {
      feed: {
        mentions,
        needs_action: needsAction,
        activity,
        agent_activity: agentActivity,
      },
      meta: {
        since: args.since ?? now - 7 * 24 * 60 * 60,
        total:
          mentions.length +
          needsAction.length +
          activity.length +
          agentActivity.length,
        generated_at: now,
      },
    };
  }

  // Feed is composed of multiple queries: mentions (#p), activity, approvals.
  // For e2e, return a minimal feed structure with mentions.
  const limit = args.limit ?? 50;
  const mentionEvents = await relayQuery(config, [
    { kinds: [9, 40002, 45001, 45003], "#p": [identity.pubkey], limit },
  ]);

  // Look up channel names for feed items
  const channelIdsInFeed = [
    ...new Set(
      mentionEvents
        .map(
          (ev) =>
            ((ev.tags ?? []) as string[][]).find((t) => t[0] === "h")?.[1],
        )
        .filter(Boolean) as string[],
    ),
  ];
  const channelNameMap = new Map<string, string>();
  if (channelIdsInFeed.length > 0) {
    const metaEvents = await relayQuery(config, [
      {
        kinds: [39000],
        "#d": channelIdsInFeed,
        limit: channelIdsInFeed.length,
      },
    ]);
    for (const me of metaEvents) {
      const d = ((me.tags ?? []) as string[][]).find((t) => t[0] === "d")?.[1];
      const name = ((me.tags ?? []) as string[][]).find(
        (t) => t[0] === "name",
      )?.[1];
      if (d && name) channelNameMap.set(d, name);
    }
  }

  const items = mentionEvents.map((ev) => {
    const chId =
      ((ev.tags ?? []) as string[][]).find((t) => t[0] === "h")?.[1] ?? null;
    return {
      id: ev.id ?? "",
      pubkey: ev.pubkey ?? "",
      content: ev.content ?? "",
      created_at: ev.created_at ?? 0,
      kind: ev.kind ?? 9,
      tags: (ev.tags ?? []) as string[][],
      channel_id: chId,
      channel_name: chId ? (channelNameMap.get(chId) ?? "") : "",
      category: "mention" as const,
    };
  });
  return {
    feed: {
      mentions: items,
      needs_action: [],
      activity: [],
      agent_activity: [],
    },
    meta: {
      since: Math.floor(Date.now() / 1000) - 7 * 86400,
      total: items.length,
      generated_at: Math.floor(Date.now() / 1000),
    },
  };
}

async function handleListRelayAgents(): Promise<RawRelayAgent[]> {
  syncMockRelayAgentsFromManagedAgents();
  return mockRelayAgents.map(cloneRelayAgent);
}

async function handleDiscoverAcpProviders(
  config: E2eConfig | undefined,
): Promise<RawAcpProvider[]> {
  const configuredProviders = config?.mock?.acpProviders;
  if (configuredProviders) {
    return configuredProviders.map((provider) => ({
      id: provider.id,
      label: provider.label,
      command: provider.command,
      binary_path: provider.binaryPath,
      default_args: [...provider.defaultArgs],
      mcp_command: provider.mcpCommand ?? null,
    }));
  }

  return [
    {
      id: "goose",
      label: "Goose",
      command: "goose",
      binary_path: "/usr/local/bin/goose",
      default_args: ["acp"],
      mcp_command: null,
    },
    {
      id: "codex",
      label: "Codex",
      command: "codex-acp",
      binary_path: "/usr/local/bin/codex-acp",
      default_args: [],
      mcp_command: null,
    },
  ];
}

async function handleDiscoverManagedAgentPrereqs(
  args: {
    input?: {
      acpCommand?: string;
      mcpCommand?: string;
    };
  },
  config: E2eConfig | undefined,
): Promise<RawManagedAgentPrereqs> {
  const configuredPrereqs = config?.mock?.managedAgentPrereqs;

  return {
    acp: {
      command:
        configuredPrereqs?.acp?.command ??
        args.input?.acpCommand ??
        "sprout-acp",
      resolved_path:
        configuredPrereqs?.acp?.resolvedPath ??
        "/Users/wesb/dev/sprout/target/debug/sprout-acp",
      available: configuredPrereqs?.acp?.available ?? true,
    },
    mcp: {
      command:
        configuredPrereqs?.mcp?.command ??
        args.input?.mcpCommand ??
        "sprout-mcp-server",
      resolved_path:
        configuredPrereqs?.mcp?.resolvedPath ??
        "/Users/wesb/dev/sprout/target/debug/sprout-mcp-server",
      available: configuredPrereqs?.mcp?.available ?? true,
    },
  };
}

async function handleListManagedAgents(): Promise<RawManagedAgent[]> {
  return mockManagedAgents.map(cloneManagedAgent);
}

async function handleListPersonas(): Promise<RawPersona[]> {
  return mockPersonas.map((persona) => ({ ...persona }));
}

async function handleCreatePersona(args: {
  input: {
    displayName: string;
    avatarUrl?: string;
    systemPrompt: string;
    envVars?: Record<string, string>;
  };
}): Promise<RawPersona> {
  const now = new Date().toISOString();
  const persona: RawPersona = {
    id: crypto.randomUUID(),
    display_name: args.input.displayName.trim(),
    avatar_url: args.input.avatarUrl?.trim() || null,
    system_prompt: args.input.systemPrompt.trim(),
    is_builtin: false,
    is_active: true,
    env_vars: { ...(args.input.envVars ?? {}) },
    created_at: now,
    updated_at: now,
  };
  mockPersonas.push(persona);
  return { ...persona };
}

async function handleUpdatePersona(args: {
  input: {
    id: string;
    displayName: string;
    avatarUrl?: string;
    systemPrompt: string;
    envVars?: Record<string, string>;
  };
}): Promise<RawPersona> {
  const persona = mockPersonas.find(
    (candidate) => candidate.id === args.input.id,
  );
  if (!persona) {
    throw new Error(`Persona ${args.input.id} not found.`);
  }
  if (persona.is_builtin) {
    throw new Error("Built-in personas cannot be edited.");
  }

  persona.display_name = args.input.displayName.trim();
  persona.avatar_url = args.input.avatarUrl?.trim() || null;
  persona.system_prompt = args.input.systemPrompt.trim();
  if (args.input.envVars !== undefined) {
    // Absent = preserve; present = replace entirely (matches Rust handler).
    persona.env_vars = { ...args.input.envVars };
  }
  persona.updated_at = new Date().toISOString();

  return { ...persona };
}

async function handleDeletePersona(args: { id: string }): Promise<void> {
  const persona = mockPersonas.find((candidate) => candidate.id === args.id);
  if (!persona) {
    throw new Error(`Persona ${args.id} not found.`);
  }
  if (persona.is_builtin) {
    throw new Error("Built-in personas cannot be deleted.");
  }
  if (mockTeams.some((team) => team.persona_ids.includes(args.id))) {
    throw new Error(
      `${persona.display_name} is still referenced by a team. Remove it from those teams first.`,
    );
  }

  mockPersonas = mockPersonas.filter((candidate) => candidate.id !== args.id);
  const now = new Date().toISOString();
  for (const agent of mockManagedAgents) {
    if (agent.persona_id === args.id) {
      agent.persona_id = null;
      agent.updated_at = now;
    }
  }
}

async function handleSetPersonaActive(args: {
  id: string;
  active: boolean;
}): Promise<RawPersona> {
  const persona = mockPersonas.find((candidate) => candidate.id === args.id);
  if (!persona) {
    throw new Error(`Persona ${args.id} not found.`);
  }
  if (!persona.is_builtin) {
    throw new Error(
      "Only built-in personas can be added to or removed from My Agents.",
    );
  }
  if (
    !args.active &&
    mockManagedAgents.some((agent) => agent.persona_id === args.id)
  ) {
    throw new Error(
      `${persona.display_name} is still assigned to a managed agent. Remove or reassign those agents first.`,
    );
  }
  if (
    !args.active &&
    mockTeams.some((team) => team.persona_ids.includes(args.id))
  ) {
    throw new Error(
      `${persona.display_name} is still referenced by a team. Remove it from those teams first.`,
    );
  }

  persona.is_active = args.active;
  persona.updated_at = new Date().toISOString();
  return { ...persona };
}

function ensureMockPersonaIsActive(personaId: string) {
  const persona = mockPersonas.find((candidate) => candidate.id === personaId);
  if (!persona) {
    throw new Error(`persona ${personaId} not found`);
  }
  if (!persona.is_active) {
    throw new Error(
      `${persona.display_name} is not in My Agents. Choose it from Persona Catalog first.`,
    );
  }
}

function ensureMockPersonaIdsAreActive(personaIds: string[]) {
  for (const personaId of personaIds) {
    ensureMockPersonaIsActive(personaId);
  }
}

async function handleListTeams(): Promise<RawTeam[]> {
  return mockTeams.map((team) => ({
    ...team,
    persona_ids: [...team.persona_ids],
  }));
}

async function handleCreateTeam(args: {
  input: {
    name: string;
    description?: string;
    personaIds: string[];
  };
}): Promise<RawTeam> {
  ensureMockPersonaIdsAreActive(args.input.personaIds);
  const now = new Date().toISOString();
  const team: RawTeam = {
    id: crypto.randomUUID(),
    name: args.input.name.trim(),
    description: args.input.description?.trim() || null,
    persona_ids: [...args.input.personaIds],
    is_builtin: false,
    created_at: now,
    updated_at: now,
  };
  mockTeams.push(team);
  return { ...team, persona_ids: [...team.persona_ids] };
}

async function handleUpdateTeam(args: {
  input: {
    id: string;
    name: string;
    description?: string;
    personaIds: string[];
  };
}): Promise<RawTeam> {
  const team = mockTeams.find((candidate) => candidate.id === args.input.id);
  if (!team) {
    throw new Error(`Team ${args.input.id} not found.`);
  }

  ensureMockPersonaIdsAreActive(args.input.personaIds);
  team.name = args.input.name.trim();
  team.description = args.input.description?.trim() || null;
  team.persona_ids = [...args.input.personaIds];
  team.updated_at = new Date().toISOString();

  return { ...team, persona_ids: [...team.persona_ids] };
}

async function handleDeleteTeam(args: { id: string }): Promise<void> {
  const team = mockTeams.find((candidate) => candidate.id === args.id);
  if (team?.is_builtin) {
    throw new Error("Built-in teams cannot be deleted.");
  }
  mockTeams = mockTeams.filter((candidate) => candidate.id !== args.id);
}

async function handleExportTeamToJson(args: { id: string }): Promise<boolean> {
  const team = mockTeams.find((candidate) => candidate.id === args.id);
  if (!team) {
    throw new Error(`Team ${args.id} not found.`);
  }

  const missingPersonaIds = team.persona_ids.filter(
    (personaId) =>
      !mockPersonas.some((candidate) => candidate.id === personaId),
  );
  if (missingPersonaIds.length > 0) {
    throw new Error(
      `Team ${team.name} references missing personas: ${missingPersonaIds.join(", ")}. Repair the team before exporting.`,
    );
  }

  return true;
}

async function handleParseTeamFile(): Promise<{
  name: string;
  description: string | null;
  personas: Array<{
    display_name: string;
    system_prompt: string;
    avatar_url: string | null;
  }>;
}> {
  return {
    name: "Imported Team",
    description: null,
    personas: [],
  };
}

async function handleParsePersonaFiles(args: {
  fileBytes: number[];
  fileName: string;
}): Promise<{
  personas: {
    display_name: string;
    system_prompt: string;
    avatar_data_url: string | null;
    source_file: string;
  }[];
  skipped: { source_file: string; reason: string }[];
}> {
  // In test mode, return canned data — we can't actually parse PNG chunks in JS
  return {
    personas: [
      {
        display_name: "Imported Persona",
        system_prompt: "You are an imported test persona.",
        avatar_data_url: null,
        source_file: args.fileName,
      },
    ],
    skipped: [],
  };
}

async function handleExportPersonaToJson(args: {
  id: string;
}): Promise<boolean> {
  // In test mode, just verify the persona exists
  const persona = mockPersonas.find((p) => p.id === args.id);
  if (!persona) throw new Error(`Persona ${args.id} not found.`);
  return true; // Simulate successful save
}

async function handleCreateManagedAgent(args: {
  input: {
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
    envVars?: Record<string, string>;
    spawnAfterCreate?: boolean;
    startOnAppLaunch?: boolean;
    backend?:
      | { type: "local" }
      | { type: "provider"; id: string; config: Record<string, unknown> };
    respondTo?: "owner-only" | "allowlist" | "anyone";
    respondToAllowlist?: string[];
  };
}): Promise<RawCreateManagedAgentResponse> {
  if (args.input.personaId) {
    ensureMockPersonaIsActive(args.input.personaId);
  }
  const name = args.input.name.trim();
  const now = new Date().toISOString();
  const pubkey = crypto
    .randomUUID()
    .replace(/-/g, "")
    .padEnd(64, "0")
    .slice(0, 64);
  const managedAgent: MockManagedAgent = {
    pubkey,
    name,
    persona_id: args.input.personaId ?? null,
    relay_url: args.input.relayUrl ?? DEFAULT_RELAY_WS_URL,
    acp_command: args.input.acpCommand ?? "sprout-acp",
    agent_command: args.input.agentCommand ?? "goose",
    agent_args:
      args.input.agentArgs && args.input.agentArgs.length > 0
        ? [...args.input.agentArgs]
        : ["acp"],
    mcp_command: args.input.mcpCommand ?? "sprout-mcp-server",
    turn_timeout_seconds: args.input.turnTimeoutSeconds ?? 320,
    idle_timeout_seconds: args.input.idleTimeoutSeconds ?? null,
    max_turn_duration_seconds: args.input.maxTurnDurationSeconds ?? null,
    parallelism: args.input.parallelism ?? 1,
    system_prompt: args.input.systemPrompt?.trim() || null,
    model: args.input.model?.trim() || null,
    env_vars: { ...(args.input.envVars ?? {}) },
    status: args.input.spawnAfterCreate ? "running" : "stopped",
    pid: args.input.spawnAfterCreate ? 42000 + mockManagedAgents.length : null,
    created_at: now,
    updated_at: now,
    last_started_at: args.input.spawnAfterCreate ? now : null,
    last_stopped_at: null,
    last_exit_code: null,
    last_error: null,
    log_path: `/tmp/mock-agent-${pubkey}.log`,
    start_on_app_launch: args.input.startOnAppLaunch ?? true,
    backend: args.input.backend ?? { type: "local" as const },
    backend_agent_id: null,
    respond_to: args.input.respondTo ?? "owner-only",
    respond_to_allowlist: args.input.respondToAllowlist ?? [],
    private_key_nsec: `nsec1mock${pubkey.slice(0, 20)}`,
    log_lines: [
      `sprout-acp starting: relay=${args.input.relayUrl ?? DEFAULT_RELAY_WS_URL} agent_pubkey=${pubkey} parallelism=${args.input.parallelism ?? 1}`,
      args.input.systemPrompt?.trim()
        ? `system prompt override configured (${args.input.systemPrompt.trim().length} chars)`
        : "system prompt override not set",
      args.input.spawnAfterCreate
        ? "connected to relay at ws://localhost:3000"
        : "profile created; harness not started",
    ],
  };

  mockManagedAgents.unshift(managedAgent);
  syncMockRelayAgentsFromManagedAgents();

  return {
    agent: cloneManagedAgent(managedAgent),
    private_key_nsec: managedAgent.private_key_nsec,
    profile_sync_error: null,
    spawn_error: null,
  };
}

function getMockManagedAgent(pubkey: string): MockManagedAgent {
  const agent = mockManagedAgents.find(
    (candidate) => candidate.pubkey === pubkey,
  );
  if (!agent) {
    throw new Error(`Managed agent ${pubkey} not found.`);
  }

  return agent;
}

async function handleStartManagedAgent(args: {
  pubkey: string;
}): Promise<RawManagedAgent> {
  const agent = getMockManagedAgent(args.pubkey);
  const now = new Date().toISOString();
  agent.status = "running";
  agent.pid = agent.pid ?? 42000 + mockManagedAgents.indexOf(agent);
  agent.updated_at = now;
  agent.last_started_at = now;
  agent.last_error = null;
  agent.log_lines.push(`started mock harness at ${now}`);
  syncMockRelayAgentsFromManagedAgents();
  return cloneManagedAgent(agent);
}

async function handleStopManagedAgent(args: {
  pubkey: string;
}): Promise<RawManagedAgent> {
  const agent = getMockManagedAgent(args.pubkey);
  const now = new Date().toISOString();
  agent.status = "stopped";
  agent.pid = null;
  agent.updated_at = now;
  agent.last_stopped_at = now;
  agent.log_lines.push(`stopped mock harness at ${now}`);
  syncMockRelayAgentsFromManagedAgents();
  return cloneManagedAgent(agent);
}

async function handleDeleteManagedAgent(args: {
  pubkey: string;
  forceRemoteDelete?: boolean | null;
}): Promise<void> {
  // Model the backend invariant: reject deletion of deployed remote agents
  // unless force_remote_delete is true.
  const agent = mockManagedAgents.find((a) => a.pubkey === args.pubkey);
  if (
    agent &&
    agent.backend.type === "provider" &&
    agent.backend_agent_id != null &&
    !args.forceRemoteDelete
  ) {
    throw new Error(
      "cannot delete a deployed remote agent without force_remote_delete: true",
    );
  }
  mockManagedAgents = mockManagedAgents.filter(
    (candidate) => candidate.pubkey !== args.pubkey,
  );
  syncMockRelayAgentsFromManagedAgents();
}

async function handleSetManagedAgentStartOnAppLaunch(args: {
  pubkey: string;
  startOnAppLaunch: boolean;
}): Promise<RawManagedAgent> {
  const agent = getMockManagedAgent(args.pubkey);
  agent.start_on_app_launch = args.startOnAppLaunch;
  agent.updated_at = new Date().toISOString();
  return cloneManagedAgent(agent);
}

async function handleGetManagedAgentLog(args: {
  pubkey: string;
  lineCount?: number;
}): Promise<RawManagedAgentLog> {
  const agent = getMockManagedAgent(args.pubkey);
  const count = args.lineCount ?? 120;
  return {
    content: agent.log_lines.slice(-count).join("\n"),
    log_path: agent.log_path,
  };
}

async function handleUpdateManagedAgent(args: {
  input: {
    pubkey: string;
    name?: string;
    model?: string | null;
    systemPrompt?: string | null;
    envVars?: Record<string, string>;
    respondTo?: "owner-only" | "allowlist" | "anyone";
    respondToAllowlist?: string[];
  };
}): Promise<{ agent: RawManagedAgent; profile_sync_error: string | null }> {
  const agent = getMockManagedAgent(args.input.pubkey);
  if (args.input.name !== undefined) {
    agent.name = args.input.name;
  }
  if (args.input.model !== undefined) {
    agent.model = args.input.model;
  }
  if (args.input.systemPrompt !== undefined) {
    agent.system_prompt = args.input.systemPrompt;
  }
  if (args.input.envVars !== undefined) {
    agent.env_vars = { ...args.input.envVars };
  }
  if (args.input.respondTo !== undefined) {
    agent.respond_to = args.input.respondTo;
  }
  if (args.input.respondToAllowlist !== undefined) {
    agent.respond_to_allowlist = args.input.respondToAllowlist;
  }
  agent.updated_at = new Date().toISOString();
  return { agent: cloneManagedAgent(agent), profile_sync_error: null };
}

async function handleSearchMessages(
  args: {
    q: string;
    limit?: number;
  },
  config: E2eConfig | undefined,
): Promise<RawSearchResponse> {
  const identity = getIdentity(config);
  if (!identity) {
    const query = args.q.trim().toLowerCase();
    const limit = args.limit ?? 20;
    const now = Math.floor(Date.now() / 1000);

    const mockHits: RawSearchHit[] = [
      {
        event_id: "mock-general-welcome",
        content: "Welcome to #general",
        kind: 9,
        pubkey: DEFAULT_MOCK_IDENTITY.pubkey,
        channel_id: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
        channel_name: "general",
        created_at: now - 60,
        score: 8.5,
      },
      {
        event_id: "mock-engineering-shipped",
        content: "Engineering shipped the desktop build.",
        kind: 9,
        pubkey:
          "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260",
        channel_id: "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9",
        channel_name: "engineering",
        created_at: now - 42 * 60,
        score: 7.2,
      },
      {
        event_id: "mock-design-critique",
        content: "Design critique notes for the browse flow.",
        kind: 9,
        pubkey:
          "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
        channel_id: "b5e2f8a1-3c44-5912-9e67-4a8d1f2b3c4e",
        channel_name: "design",
        created_at: now - 75 * 60,
        score: 6.6,
      },
      {
        event_id: "mock-forum-release-thread",
        content: "Release checklist: async feedback thread.",
        kind: 45001,
        pubkey:
          "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
        channel_id: "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11",
        channel_name: "watercooler",
        created_at: now - 90 * 60,
        score: 5.8,
      },
      {
        event_id: "mock-forum-release-reply",
        content: "Looks good to me. We should ship it.",
        kind: 45003,
        pubkey: ALICE_PUBKEY,
        channel_id: "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11",
        channel_name: "watercooler",
        created_at: now - 80 * 60,
        score: 5.2,
      },
    ];

    const hits = mockHits
      .filter((hit) => {
        if (!query) {
          return true;
        }

        return (
          hit.content.toLowerCase().includes(query) ||
          (hit.channel_name?.toLowerCase().includes(query) ?? false)
        );
      })
      .slice(0, limit);

    return {
      hits,
      found: hits.length,
    };
  }

  // NIP-50 search via POST /query
  const limit = args.limit ?? 20;
  const events = await relayQuery(config, [
    { kinds: [9, 40002], search: args.q, limit },
  ]);
  const hits = events.map((ev) => ({
    event_id: ev.id ?? "",
    pubkey: ev.pubkey ?? "",
    content: ev.content ?? "",
    created_at: ev.created_at ?? 0,
    kind: ev.kind ?? 9,
    tags: ev.tags ?? [],
    sig: ev.sig ?? "",
    channel_id:
      ((ev.tags ?? []) as string[][]).find((t) => t[0] === "h")?.[1] ?? null,
    channel_name: null,
    score: 1.0,
  }));
  return { hits, found: hits.length };
}

async function handleSendChannelMessage(
  args: {
    channelId: string;
    content: string;
    parentEventId?: string | null;
    kind?: number | null;
    mentionPubkeys?: string[];
  },
  config: E2eConfig | undefined,
): Promise<RawSendChannelMessageResponse> {
  const kind = args.kind ?? 9;
  const identity = getIdentity(config);
  if (!identity) {
    const createdAt = Math.floor(Date.now() / 1000);
    const mockPubkey = getMockMemberPubkey(config);

    if (!args.parentEventId) {
      const event = createMockEvent(
        kind,
        args.content,
        buildTopLevelMessageTags(
          args.channelId,
          args.mentionPubkeys,
          mockPubkey,
        ),
      );
      recordMockMessage(args.channelId, event);
      emitMockLiveEvent(args.channelId, event);

      return {
        event_id: event.id,
        parent_event_id: null,
        root_event_id: null,
        depth: 0,
        created_at: createdAt,
      };
    }

    const history = getMockMessageStore(args.channelId);
    const parentEvent = history.find(
      (event) => event.id === args.parentEventId,
    );
    const parentThread = parentEvent
      ? getThreadReferenceFromTags(parentEvent.tags)
      : {
          parentEventId: null,
          rootEventId: null,
        };
    const rootEventId = parentThread.rootEventId ?? args.parentEventId;
    const depth = parentEvent
      ? (() => {
          let currentEvent: RelayEvent | undefined = parentEvent;
          let nextDepth = 1;

          while (currentEvent) {
            const reference = getThreadReferenceFromTags(currentEvent.tags);
            if (!reference.parentEventId) {
              return nextDepth;
            }

            nextDepth += 1;
            currentEvent = history.find(
              (event) => event.id === reference.parentEventId,
            );
          }

          return nextDepth;
        })()
      : 1;

    const event: RelayEvent = {
      id: crypto.randomUUID().replace(/-/g, ""),
      pubkey: mockPubkey,
      created_at: createdAt,
      kind,
      tags: buildReplyMessageTags(
        args.channelId,
        mockPubkey,
        args.parentEventId,
        rootEventId,
        args.mentionPubkeys,
      ),
      content: args.content.trim(),
      sig: "mocksig".repeat(20).slice(0, 128),
    };

    recordMockMessage(args.channelId, event);
    emitMockLiveEvent(args.channelId, event);

    return {
      event_id: event.id,
      parent_event_id: args.parentEventId,
      root_event_id: rootEventId,
      depth,
      created_at: createdAt,
    };
  }

  const relayIdentity = getRelayIdentity(config);
  const tags = args.parentEventId
    ? buildReplyMessageTags(
        args.channelId,
        relayIdentity.pubkey,
        args.parentEventId,
        args.parentEventId,
        args.mentionPubkeys,
      )
    : buildTopLevelMessageTags(
        args.channelId,
        args.mentionPubkeys,
        relayIdentity.pubkey,
      );

  const result = await submitSignedEvent(config, {
    kind,
    content: args.content.trim(),
    tags,
  });

  return {
    event_id: result.event_id,
    parent_event_id: args.parentEventId ?? null,
    root_event_id: args.parentEventId ?? null,
    depth: args.parentEventId ? 1 : 0,
    created_at: Math.floor(Date.now() / 1000),
  };
}

async function handleGetEvent(
  args: {
    eventId: string;
  },
  config: E2eConfig | undefined,
) {
  const identity = getIdentity(config);
  if (!identity) {
    const knownEvents: RelayEvent[] = [
      ...Array.from(mockMessages.values()).flat(),
      {
        id: "mock-engineering-shipped",
        pubkey:
          "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260",
        created_at: Math.floor(Date.now() / 1000) - 42 * 60,
        kind: 9,
        tags: [["e", "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9"]],
        content: "Engineering shipped the desktop build.",
        sig: "mocksig".repeat(20).slice(0, 128),
      },
      {
        id: "mock-design-critique",
        pubkey:
          "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
        created_at: Math.floor(Date.now() / 1000) - 75 * 60,
        kind: 9,
        tags: [["h", "b5e2f8a1-3c44-5912-9e67-4a8d1f2b3c4e"]],
        content: "Design critique notes for the browse flow.",
        sig: "mocksig".repeat(20).slice(0, 128),
      },
      {
        id: "mock-forum-release-thread",
        pubkey:
          "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
        created_at: Math.floor(Date.now() / 1000) - 90 * 60,
        kind: 45001,
        tags: [["e", "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11"]],
        content: "Release checklist: async feedback thread.",
        sig: "mocksig".repeat(20).slice(0, 128),
      },
      {
        id: "mock-forum-release-reply",
        pubkey: ALICE_PUBKEY,
        created_at: Math.floor(Date.now() / 1000) - 80 * 60,
        kind: 45003,
        tags: buildReplyMessageTags(
          "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11",
          ALICE_PUBKEY,
          "mock-forum-release-thread",
          "mock-forum-release-thread",
          undefined,
        ),
        content: "Looks good to me. We should ship it.",
        sig: "mocksig".repeat(20).slice(0, 128),
      },
    ];
    const event = knownEvents.find((item) => item.id === args.eventId);
    if (!event) {
      throw new Error(`Event not found: ${args.eventId}`);
    }

    return JSON.stringify(event);
  }

  // Query single event by ID via POST /query
  const events = await relayQuery(config, [{ ids: [args.eventId], limit: 1 }]);
  if (events.length === 0) {
    throw new Error(`Event not found: ${args.eventId}`);
  }
  return JSON.stringify(events[0]);
}

async function connectRealSocket(args: { url?: string; onMessage: unknown }) {
  const wsId = nextSocketId++;
  const ws = new WebSocket(args.url ?? DEFAULT_RELAY_WS_URL);
  const handler = resolveHandler(args.onMessage);

  realSockets.set(wsId, ws);
  ws.addEventListener("message", (event) => {
    handler({
      type: "Text",
      data: event.data,
    });
  });
  ws.addEventListener("close", () => {
    sendWsClose(handler);
    realSockets.delete(wsId);
  });
  ws.addEventListener("error", () => {
    handler({
      type: "Error",
    });
  });

  return await new Promise<number>((resolve) => {
    ws.addEventListener("open", () => resolve(wsId), { once: true });
    ws.addEventListener("error", () => resolve(wsId), { once: true });
  });
}

async function connectMockSocket(args: { onMessage: unknown }) {
  const wsId = nextSocketId++;
  const handler = resolveHandler(args.onMessage);

  mockSockets.set(wsId, {
    handler,
    subscriptions: new Map(),
  });

  window.setTimeout(() => {
    sendWsText(handler, ["AUTH", `mock-challenge-${wsId}`]);
  }, 0);

  return wsId;
}

async function sendToRealSocket(args: {
  id: number;
  message?: {
    type: "Text" | "Close";
    data?: string;
  };
}) {
  const socket = realSockets.get(args.id);
  if (!socket) {
    return;
  }

  if (args.message?.type === "Close") {
    socket.close();
    return;
  }

  if (args.message?.type === "Text") {
    socket.send(args.message.data ?? "");
  }
}

function sendToMockSocket(args: {
  id: number;
  message?: {
    type: "Text" | "Close";
    data?: string;
  };
}) {
  const socket = mockSockets.get(args.id);
  if (!socket || !args.message) {
    return;
  }

  if (args.message.type === "Close") {
    mockSockets.delete(args.id);
    sendWsClose(socket.handler);
    return;
  }

  if (args.message.type !== "Text" || !args.message.data) {
    return;
  }

  const [type, ...rest] = JSON.parse(args.message.data) as [
    string,
    ...unknown[],
  ];

  if (type === "AUTH") {
    const event = rest[0] as RelayEvent;
    sendWsText(socket.handler, ["OK", event.id, true, ""]);
    return;
  }

  if (type === "REQ") {
    const subId = rest[0] as string;

    if (subId.startsWith("live-")) {
      // Collect channel IDs from all filters in the REQ
      const channelIds = new Set<string>();
      const kinds = new Set<number>();
      for (let i = 1; i < rest.length; i++) {
        const f = rest[i] as { "#h"?: string[]; kinds?: number[] };
        const cid = f["#h"]?.[0];
        if (cid) channelIds.add(cid);
        for (const kind of f.kinds ?? []) {
          kinds.add(kind);
        }
      }
      const onlyChannelId =
        channelIds.size === 1
          ? (channelIds.values().next().value as string)
          : undefined;
      socket.subscriptions.set(subId, {
        channelId: onlyChannelId ?? GLOBAL_MOCK_SUBSCRIPTION,
        kinds: kinds.size > 0 ? [...kinds] : null,
      });
      sendWsText(socket.handler, ["EOSE", subId]);
      return;
    }

    const filter = rest[1] as { "#h"?: string[]; kinds?: number[] };
    if (filter.kinds?.includes(13534)) {
      sendWsText(socket.handler, [
        "EVENT",
        subId,
        createMockRelayMembershipEvent(),
      ]);
      sendWsText(socket.handler, ["EOSE", subId]);
      return;
    }

    const channelId = filter["#h"]?.[0];
    if (!channelId) {
      sendWsText(socket.handler, ["EOSE", subId]);
      return;
    }

    emitMockHistory(socket, subId, channelId);
    return;
  }

  if (type === "CLOSE") {
    const subId = rest[0] as string;
    socket.subscriptions.delete(subId);
    return;
  }

  if (type === "EVENT") {
    const event = rest[0] as RelayEvent;

    if ([9030, 9031, 9032].includes(event.kind)) {
      const accepted = updateMockRelayMembershipFromAdminEvent(event);
      sendWsText(socket.handler, [
        "OK",
        event.id,
        accepted,
        accepted ? "" : "Invalid relay admin event.",
      ]);
      return;
    }

    const channelId = getChannelIdFromTags(event.tags);
    if (!channelId) {
      sendWsText(socket.handler, [
        "OK",
        event.id,
        false,
        "Missing channel tag.",
      ]);
      return;
    }

    recordMockMessage(channelId, event);
    emitMockLiveEvent(channelId, event);
    sendWsText(socket.handler, ["OK", event.id, true, ""]);
  }
}

function disconnectMockSocket(id: number) {
  const socket = mockSockets.get(id);
  if (!socket) {
    return;
  }

  mockSockets.delete(id);
  sendWsClose(socket.handler);
}

export function maybeInstallE2eTauriMocks() {
  if (installed) {
    return;
  }

  const config = getConfig();
  if (!config) {
    return;
  }

  resetMockRelayMembers(config);
  resetMockManagedAgents();
  resetMockPersonas();
  resetMockTeams();
  resetMockWorkflows();
  mockWindows("main");
  window.__SPROUT_E2E_COMMANDS__ = [];
  window.__SPROUT_E2E_WEBVIEW_ZOOM__ = 1;
  window.__SPROUT_E2E_EMIT_MOCK_MESSAGE__ = ({
    channelName,
    content,
    parentEventId,
    pubkey,
    kind,
  }) => {
    const channel = mockChannels.find(
      (candidate) => candidate.name === channelName,
    );
    if (!channel) {
      throw new Error(`Mock channel ${channelName} not found.`);
    }

    return emitMockChannelMessage(
      channel.id,
      content,
      parentEventId,
      pubkey,
      kind,
    );
  };
  window.__SPROUT_E2E_EMIT_MOCK_TYPING__ = ({ channelName, pubkey }) => {
    const channel = mockChannels.find(
      (candidate) => candidate.name === channelName,
    );
    if (!channel) {
      throw new Error(`Mock channel ${channelName} not found.`);
    }

    return emitMockTypingIndicator(channel.id, pubkey ?? CHARLIE_PUBKEY);
  };
  window.__SPROUT_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__ = ({
    channelName,
    kind,
  }) => {
    const channel = mockChannels.find(
      (candidate) => candidate.name === channelName,
    );
    if (!channel) {
      throw new Error(`Mock channel ${channelName} not found.`);
    }

    return hasMockLiveSubscription(channel.id, kind);
  };
  window.__SPROUT_E2E_PUSH_MOCK_FEED_ITEM__ = (item) => {
    const category = item.category === "mention" ? "mentions" : item.category;
    mockFeedOverrides[category].unshift(item);
    window.dispatchEvent(new CustomEvent("sprout:e2e-home-feed-updated"));
    return item;
  };
  const handleMockCommand = async (command: string, payload: unknown) => {
    const activeConfig = getConfig();
    const identity = getActiveIdentity(activeConfig);
    window.__SPROUT_E2E_COMMANDS__?.push(command);

    switch (command) {
      case "get_identity":
        if (identity) {
          return {
            pubkey: identity.pubkey,
            display_name: identity.username,
          };
        }

        return DEFAULT_MOCK_IDENTITY;
      case "get_nsec":
        return "nsec1mock000000000000000000000000000000000000000000000000000000";
      case "apply_workspace":
        return;
      case "get_profile":
        return handleGetProfile(activeConfig);
      case "update_profile":
        return handleUpdateProfile(
          payload as Parameters<typeof handleUpdateProfile>[0],
          activeConfig,
        );
      case "get_user_profile":
        return handleGetUserProfile(
          (payload as Parameters<typeof handleGetUserProfile>[0]) ?? {},
          activeConfig,
        );
      case "get_users_batch":
        return handleGetUsersBatch(
          payload as Parameters<typeof handleGetUsersBatch>[0],
          activeConfig,
        );
      case "get_user_notes":
        return handleGetUserNotes(
          payload as Parameters<typeof handleGetUserNotes>[0],
          activeConfig,
        );
      case "search_users":
        return handleSearchUsers(
          payload as Parameters<typeof handleSearchUsers>[0],
          activeConfig,
        );
      case "get_presence":
        return handleGetPresence(
          (payload as Parameters<typeof handleGetPresence>[0]) ?? {
            pubkeys: [],
          },
          activeConfig,
        );
      case "set_presence":
        return handleSetPresence(
          payload as Parameters<typeof handleSetPresence>[0],
          activeConfig,
        );
      case "get_relay_ws_url":
        return getRelayWsUrl(activeConfig);
      case "get_default_relay_url":
        return getRelayWsUrl(activeConfig);
      case "get_relay_http_url":
        return getRelayHttpUrl(activeConfig);
      case "discover_acp_providers":
        return handleDiscoverAcpProviders(activeConfig);
      case "discover_backend_providers":
        return [];
      case "probe_backend_provider":
        return { ok: false, error: "mock: no providers available" };
      case "discover_managed_agent_prereqs":
        return handleDiscoverManagedAgentPrereqs(
          payload as Parameters<typeof handleDiscoverManagedAgentPrereqs>[0],
          activeConfig,
        );
      case "get_channels":
        return handleGetChannels(activeConfig);
      case "get_feed":
        return handleGetFeed(
          (payload as Parameters<typeof handleGetFeed>[0]) ?? {},
          activeConfig,
        );
      case "list_relay_agents":
        return handleListRelayAgents();
      case "list_personas":
        return handleListPersonas();
      case "create_persona":
        return handleCreatePersona(
          payload as Parameters<typeof handleCreatePersona>[0],
        );
      case "update_persona":
        return handleUpdatePersona(
          payload as Parameters<typeof handleUpdatePersona>[0],
        );
      case "delete_persona":
        return handleDeletePersona(
          payload as Parameters<typeof handleDeletePersona>[0],
        );
      case "set_persona_active":
        return handleSetPersonaActive(
          payload as Parameters<typeof handleSetPersonaActive>[0],
        );
      case "list_teams":
        return handleListTeams();
      case "create_team":
        return handleCreateTeam(
          payload as Parameters<typeof handleCreateTeam>[0],
        );
      case "update_team":
        return handleUpdateTeam(
          payload as Parameters<typeof handleUpdateTeam>[0],
        );
      case "delete_team":
        return handleDeleteTeam(
          payload as Parameters<typeof handleDeleteTeam>[0],
        );
      case "export_team_to_json":
        return handleExportTeamToJson(payload as { id: string });
      case "parse_team_file":
        return handleParseTeamFile();
      case "parse_persona_files":
        return handleParsePersonaFiles(
          payload as { fileBytes: number[]; fileName: string },
        );
      case "export_persona_to_json":
        return handleExportPersonaToJson(payload as { id: string });
      case "list_managed_agents":
        return handleListManagedAgents();
      case "create_managed_agent":
        return handleCreateManagedAgent(
          payload as Parameters<typeof handleCreateManagedAgent>[0],
        );
      case "start_managed_agent":
        return handleStartManagedAgent(
          payload as Parameters<typeof handleStartManagedAgent>[0],
        );
      case "stop_managed_agent":
        return handleStopManagedAgent(
          payload as Parameters<typeof handleStopManagedAgent>[0],
        );
      case "set_managed_agent_start_on_app_launch":
        return handleSetManagedAgentStartOnAppLaunch(
          payload as Parameters<
            typeof handleSetManagedAgentStartOnAppLaunch
          >[0],
        );
      case "delete_managed_agent":
        return handleDeleteManagedAgent(
          payload as Parameters<typeof handleDeleteManagedAgent>[0],
        );
      case "get_managed_agent_log":
        return handleGetManagedAgentLog(
          payload as Parameters<typeof handleGetManagedAgentLog>[0],
        );
      case "get_agent_models":
        return {
          agentName: "mock-agent",
          agentVersion: "0.0.0",
          models: [],
          agentDefaultModel: null,
          selectedModel: null,
          supportsSwitching: false,
        };
      case "update_managed_agent":
        return handleUpdateManagedAgent(
          payload as Parameters<typeof handleUpdateManagedAgent>[0],
        );
      case "create_channel":
        return handleCreateChannel(
          payload as Parameters<typeof handleCreateChannel>[0],
          activeConfig,
        );
      case "open_dm":
        return handleOpenDm(
          payload as Parameters<typeof handleOpenDm>[0],
          activeConfig,
        );
      case "hide_dm":
        return handleHideDm(
          payload as Parameters<typeof handleHideDm>[0],
          activeConfig,
        );
      case "get_channel_details":
        return handleGetChannelDetails(
          payload as Parameters<typeof handleGetChannelDetails>[0],
          activeConfig,
        );
      case "get_channel_members":
        return handleGetChannelMembers(
          payload as Parameters<typeof handleGetChannelMembers>[0],
          activeConfig,
        );
      case "update_channel":
        return handleUpdateChannel(
          payload as Parameters<typeof handleUpdateChannel>[0],
          activeConfig,
        );
      case "set_channel_topic":
        return handleSetChannelTopic(
          payload as Parameters<typeof handleSetChannelTopic>[0],
          activeConfig,
        );
      case "set_channel_purpose":
        return handleSetChannelPurpose(
          payload as Parameters<typeof handleSetChannelPurpose>[0],
          activeConfig,
        );
      case "archive_channel":
        return handleArchiveChannel(
          payload as Parameters<typeof handleArchiveChannel>[0],
          activeConfig,
        );
      case "unarchive_channel":
        return handleUnarchiveChannel(
          payload as Parameters<typeof handleUnarchiveChannel>[0],
          activeConfig,
        );
      case "delete_channel":
        return handleDeleteChannel(
          payload as Parameters<typeof handleDeleteChannel>[0],
          activeConfig,
        );
      case "add_channel_members":
        return handleAddChannelMembers(
          payload as Parameters<typeof handleAddChannelMembers>[0],
          activeConfig,
        );
      case "remove_channel_member":
        return handleRemoveChannelMember(
          payload as Parameters<typeof handleRemoveChannelMember>[0],
          activeConfig,
        );
      case "join_channel":
        return handleJoinChannel(
          payload as Parameters<typeof handleJoinChannel>[0],
          activeConfig,
        );
      case "leave_channel":
        return handleLeaveChannel(
          payload as Parameters<typeof handleLeaveChannel>[0],
          activeConfig,
        );
      case "search_messages":
        return handleSearchMessages(
          payload as Parameters<typeof handleSearchMessages>[0],
          activeConfig,
        );
      case "get_forum_posts":
        return handleGetForumPosts(
          payload as Parameters<typeof handleGetForumPosts>[0],
        );
      case "get_forum_thread":
        return handleGetForumThread(
          payload as Parameters<typeof handleGetForumThread>[0],
        );
      case "send_channel_message":
        return handleSendChannelMessage(
          payload as Parameters<typeof handleSendChannelMessage>[0],
          activeConfig,
        );
      case "get_event":
        return handleGetEvent(
          payload as Parameters<typeof handleGetEvent>[0],
          activeConfig,
        );
      case "sign_event":
        if (identity) {
          return JSON.stringify(
            await signWithIdentity(identity, {
              kind: (payload as { kind: number }).kind,
              content: (payload as { content: string }).content,
              createdAt: (payload as { createdAt?: number }).createdAt,
              tags: (payload as { tags: string[][] }).tags,
            }),
          );
        }

        return JSON.stringify(
          createMockEvent(
            (payload as { kind: number }).kind,
            (payload as { content: string }).content,
            (payload as { tags: string[][] }).tags,
            DEFAULT_MOCK_IDENTITY.pubkey,
            (payload as { createdAt?: number }).createdAt,
          ),
        );
      case "create_auth_event":
        if (identity) {
          return JSON.stringify(
            await signWithIdentity(identity, {
              kind: 22242,
              content: "",
              tags: [
                ["relay", (payload as { relayUrl: string }).relayUrl],
                ["challenge", (payload as { challenge: string }).challenge],
              ],
            }),
          );
        }

        return JSON.stringify(
          createMockEvent(22242, "", [
            ["relay", (payload as { relayUrl: string }).relayUrl],
            ["challenge", (payload as { challenge: string }).challenge],
          ]),
        );
      case "plugin:websocket|connect":
        if (isRelayMode(activeConfig)) {
          return connectRealSocket(
            payload as Parameters<typeof connectRealSocket>[0],
          );
        }

        return connectMockSocket(
          payload as Parameters<typeof connectMockSocket>[0],
        );
      case "plugin:websocket|send":
        if (isRelayMode(activeConfig)) {
          return sendToRealSocket(
            payload as Parameters<typeof sendToRealSocket>[0],
          );
        }

        return sendToMockSocket(
          payload as Parameters<typeof sendToMockSocket>[0],
        );
      case "plugin:websocket|disconnect":
        if (isRelayMode(activeConfig)) {
          realSockets.get((payload as { id: number }).id)?.close();
          realSockets.delete((payload as { id: number }).id);
          return;
        }

        return disconnectMockSocket((payload as { id: number }).id);
      case "plugin:window|show":
      case "plugin:window|unminimize":
      case "plugin:window|set_focus":
      case "plugin:window|set_badge_count":
        return null;
      case "get_channel_workflows":
        return handleGetChannelWorkflows(
          payload as Parameters<typeof handleGetChannelWorkflows>[0],
        );
      case "get_workflow":
        return handleGetWorkflow(
          payload as Parameters<typeof handleGetWorkflow>[0],
        );
      case "create_workflow":
        return handleCreateWorkflow(
          payload as Parameters<typeof handleCreateWorkflow>[0],
        );
      case "update_workflow":
        return handleUpdateWorkflow(
          payload as Parameters<typeof handleUpdateWorkflow>[0],
        );
      case "delete_workflow":
        return handleDeleteWorkflow(
          payload as Parameters<typeof handleDeleteWorkflow>[0],
        );
      case "trigger_workflow":
        return handleTriggerWorkflow(
          payload as Parameters<typeof handleTriggerWorkflow>[0],
        );
      case "get_workflow_runs":
        return handleGetWorkflowRuns(
          payload as Parameters<typeof handleGetWorkflowRuns>[0],
        );
      case "get_run_approvals":
        return handleGetRunApprovals(
          payload as Parameters<typeof handleGetRunApprovals>[0],
        );
      case "plugin:webview|set_webview_zoom":
        window.__SPROUT_E2E_WEBVIEW_ZOOM__ = (
          payload as { value: number }
        ).value;
        return;
      case "plugin:event|listen":
        // Tauri event system (pairing, huddle) — no-op in e2e, return unlisten fn ID
        return Math.floor(Math.random() * 1_000_000);
      default:
        throw new Error(`Unsupported mocked Tauri command: ${command}`);
    }
  };
  window.__SPROUT_E2E_INVOKE_MOCK_COMMAND__ = (command, payload) =>
    handleMockCommand(command, payload ?? null);
  mockIPC(handleMockCommand);

  installed = true;
}
