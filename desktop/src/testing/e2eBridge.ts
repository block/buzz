import { bytesToHex, hexToBytes } from "@noble/hashes/utils.js";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { decode } from "nostr-tools/nip19";
import { finalizeEvent, getPublicKey } from "nostr-tools/pure";
import { parse as yamlParse } from "yaml";

import { relayClient } from "@/shared/api/relayClient";
import type { ConnectionState } from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";
import { syncAgentTurnsFromEvents } from "@/features/agents/activeAgentTurnsStore";
import {
  injectObserverEventsForE2E,
  syncAgentObserverEvents,
} from "@/features/agents/observerRelayStore";
import {
  CUSTOM_EMOJI_SET_D_TAG,
  KIND_EMOJI_SET,
} from "@/shared/api/customEmoji";
import {
  KIND_AGENT_OBSERVER_FRAME,
  KIND_DM_VISIBILITY,
  KIND_EVENT_REMINDER,
  KIND_GIT_ISSUE,
  KIND_GIT_PATCH,
  KIND_GIT_PR_UPDATE,
  KIND_GIT_PULL_REQUEST,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_OPEN,
  KIND_HUDDLE_STARTED,
  KIND_MEMBER_ADDED_NOTIFICATION,
  KIND_MEMBER_REMOVED_NOTIFICATION,
  KIND_REPO_ANNOUNCEMENT,
  KIND_STREAM_MESSAGE_EDIT,
  KIND_SYSTEM_MESSAGE,
  KIND_USER_STATUS,
} from "@/shared/constants/kinds";
import type {
  RawAcpRuntimeCatalogEntry,
  RawInstallRuntimeResult,
} from "@/shared/api/tauri";

type TestIdentity = {
  privateKey: string;
  pubkey: string;
  username: string;
};

type MockCommandAvailability = {
  available?: boolean;
  command?: string;
  resolvedPath?: string | null;
};

type MockManagedAgentSeed = {
  pubkey: string;
  name: string;
  avatarUrl?: string | null;
  personaId?: string | null;
  status?: RawManagedAgent["status"];
  channelNames?: string[];
  channelIds?: string[];
  backend?: RawManagedAgent["backend"];
  lastError?: string | null;
  respondTo?: RawManagedAgent["respond_to"];
  respondToAllowlist?: string[];
};

type MockRelayAgentSeed = {
  pubkey: string;
  name: string;
  agentType?: string;
  capabilities?: string[];
  respondTo?: RawRelayAgent["respond_to"];
  respondToAllowlist?: string[];
  channelNames?: string[];
  channelIds?: string[];
  status?: PresenceStatus;
};

type MockPersonaSeed = {
  id?: string;
  displayName: string;
  avatarUrl?: string | null;
  systemPrompt: string;
  isActive?: boolean;
  sourceTeam?: string | null;
  envVars?: Record<string, string>;
};

type MockSearchProfileSeed = {
  pubkey: string;
  displayName: string | null;
  avatarUrl?: string | null;
  nip05Handle?: string | null;
  about?: string | null;
  ownerPubkey?: string | null;
  isAgent?: boolean;
};

type E2eConfig = {
  mode?: "mock" | "relay";
  mock?: {
    acpRuntimesCatalog?: RawAcpRuntimeCatalogEntry[];
    activePersonaIds?: string[];
    installAcpRuntimeResult?: RawInstallRuntimeResult;
    managedAgentPrereqs?: {
      acp?: MockCommandAvailability;
      mcp?: MockCommandAvailability;
    };
    managedAgents?: MockManagedAgentSeed[];
    personas?: MockPersonaSeed[];
    relayAgents?: MockRelayAgentSeed[];
    agentListDelayMs?: number;
    agentMemory?: RawAgentMemoryListing | Record<string, RawAgentMemoryListing>;
    addChannelMembersDelayMs?: number;
    createManagedAgentDelayMs?: number;
    channelsReadError?: string;
    feedReadError?: string;
    canvasReadError?: string;
    /** Delay (ms) for `apply_workspace` so e2e tests can observe the
     *  workspace-switch gate. 0/undefined = instant. */
    applyWorkspaceDelayMs?: number;
    openDmDelayMs?: number;
    sendMessageDelayMs?: number;
    usersBatchDelayMs?: number;
    /** Delay (ms) applied to older-history (`history-` subId) fetches so e2e
     *  tests can observe the in-flight prepend window. 0/undefined = instant. */
    historyDelayMs?: number;
    profileReadDelayMs?: number;
    profileReadError?: string;
    profileUpdateError?: string;
    profileUpdateErrors?: string[];
    searchProfiles?: MockSearchProfileSeed[];
    updateAvailable?: boolean;
    updateChannelDelayMs?: number;
    updateDownloadDelayMs?: number;
    restartDelayMs?: number;
    updateVersion?: string;
    stallWebsocketSends?: boolean;
    userSearchDelayMs?: number;
    // NIP-IA gate inputs — see tests/helpers/bridge.ts:MockBridgeOptions for
    // semantics. These three drive the archive-button gate matrix in
    // tests/e2e/identity-archive.spec.ts; they're plumbed into:
    // - `list_archived_identities` (archivedIdentities)
    // - `resolve_oa_owner` (oaOwnerIsMe)
    // - `resetMockRelayMembers` (relayRole)
    archivedIdentities?: string[];
    oaOwnerIsMe?: boolean;
    relayRole?: "owner" | "admin" | "member" | null;
    // Descriptors returned by the mocked `pick_and_upload_media` /
    // `upload_media_bytes` commands. Lets a spec drive the attachment flow
    // (e.g. a generic PDF) without a real upload pipeline. See
    // tests/helpers/bridge.ts:MockBridgeOptions.uploadDescriptors.
    meshReporterPubkey?: string;
    uploadDelayMs?: number;
    uploadDescriptors?: RawBlobDescriptor[];
  };
  relayHttpUrl?: string;
  relayWsUrl?: string;
  identity?: TestIdentity;
};

type RawBlobDescriptor = {
  url: string;
  sha256: string;
  size: number;
  type: string;
  uploaded: number;
  dim?: string;
  blurhash?: string;
  thumb?: string;
  duration?: number;
  image?: string;
  filename?: string;
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
  owner_pubkey: string | null;
  is_agent?: boolean;
};

type RawUserProfileSummary = {
  display_name: string | null;
  avatar_url: string | null;
  nip05_handle: string | null;
  owner_pubkey: string | null;
  is_agent?: boolean;
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
  owner_pubkey: string | null;
  is_agent?: boolean;
};

type RawSearchUsersResponse = {
  users: RawUserSearchResult[];
  next_cursor?: string | null;
};

type PresenceStatus = "online" | "away" | "offline";

type RawPresenceLookup = Record<string, PresenceStatus>;

type RawChannel = {
  id: string;
  name: string;
  channel_type: "stream" | "forum" | "dm";
  visibility: "open" | "private";
  description: string;
  topic: string | null;
  purpose: string | null;
  member_count: number;
  member_pubkeys: string[];
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
  is_agent?: boolean;
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

type MockChannel = Omit<RawChannelDetail, "member_pubkeys"> & {
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
  tags: string[][];
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
  respond_to?: "owner-only" | "allowlist" | "anyone";
  respond_to_allowlist?: string[];
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
  avatar_url: string | null;
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

type RawEngramEntry = {
  slug: string;
  body: string;
  eventId: string;
  createdAt: number;
  outgoingRefs: string[];
};

type RawAgentMemoryListing = {
  core: RawEngramEntry | null;
  memories: RawEngramEntry[];
  truncated: boolean;
  fetchedAt: number;
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
  runtime?: string | null;
  model?: string | null;
  provider?: string | null;
  name_pool?: string[];
  is_builtin: boolean;
  is_active: boolean;
  source_team?: string | null;
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
  source_dir: string | null;
  is_symlink: boolean;
  symlink_target: string | null;
  version: string | null;
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

type MockFilter = {
  "#a"?: string[];
  "#d"?: string[];
  "#e"?: string[];
  "#h"?: string[];
  "#p"?: string[];
  authors?: string[];
  ids?: string[];
  kinds?: number[];
  limit?: number;
  since?: number;
  until?: number;
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

/**
 * Per-user custom emoji sets (kind:30030) the mock WS serves for
 * `listCustomEmoji` REQs. The workspace palette is the client-side UNION of
 * every member's own set (d=`buzz:custom-emoji`). We serve TWO member-authored
 * sets from distinct pubkeys so the e2e exercises the union/collapse path, not
 * a single relay-owned set. `:buzz:` is the stable shortcode exercised by
 * custom-emoji.spec.ts (claimed by BOTH members with different URLs, so the
 * palette must collapse it to one deterministic winner); `:narf:` proves a
 * second member's distinct emoji unions in.
 */
function createMockCustomEmojiSetEvents(): RelayEvent[] {
  return [
    createMockEvent(
      KIND_EMOJI_SET,
      "",
      [
        ["d", CUSTOM_EMOJI_SET_D_TAG],
        ["emoji", "buzz", "https://example.com/e2e/buzz.png"],
        // A relay-hosted emoji whose URL matches rewriteRelayUrl()'s pattern,
        // used by the reaction guard to assert the proxy rewrite fires.
        ["emoji", REACTION_EMOJI_SHORTCODE, REACTION_EMOJI_URL],
      ],
      // The current mock identity owns this set, so the settings card's
      // "My emoji" section is non-empty and removable.
      MOCK_IDENTITY_PUBKEY,
    ),
    createMockEvent(
      KIND_EMOJI_SET,
      "",
      [
        ["d", CUSTOM_EMOJI_SET_D_TAG],
        ["emoji", "narf", "https://example.com/e2e/narf.png"],
        // member B claims :buzz: with a DIFFERENT url — unionCustomEmoji must
        // collapse it to one deterministic winner, never expose two URLs.
        ["emoji", "buzz", "https://example.com/e2e/buzz-b.png"],
      ],
      "b".repeat(64),
    ),
  ];
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
    __BUZZ_E2E__?: E2eConfig;
    __BUZZ_E2E_COMMANDS__?: string[];
    __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
      command: string;
      payload: unknown;
    }>;
    __BUZZ_E2E_COMMAND_LOG__?: Array<{
      command: string;
      payload: unknown;
    }>;
    __BUZZ_E2E_WEBVIEW_ZOOM__?: number;
    __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
      channelName: string;
      kind?: number;
    }) => boolean;
    __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
      channelName: string;
      content: string;
      parentEventId?: string | null;
      pubkey?: string;
      kind?: number;
      mentionPubkeys?: string[];
      extraTags?: string[][];
      createdAt?: number;
    }) => RelayEvent;
    /** Prepend `count` synthetic older messages to a channel's mock store so
     *  an older-history fetch has something to paginate. Mirrors how the real
     *  relay backfills history. Returns the created events. */
    __BUZZ_E2E_PREPEND_MOCK_HISTORY__?: (input: {
      channelName: string;
      count: number;
      startIndex?: number;
      lineCount?: number;
      createdAtStart?: number;
      emit?: boolean;
    }) => RelayEvent[];
    __BUZZ_E2E_EMIT_MOCK_TYPING__?: (input: {
      channelName: string;
      pubkey?: string;
    }) => RelayEvent;
    __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
      command: string,
      payload?: Record<string, unknown>,
    ) => Promise<unknown>;
    __BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?: (item: RawFeedItem) => RawFeedItem;
    __BUZZ_E2E_SIGNED_EVENTS__?: Array<{
      content: string;
      kind: number;
      tags: string[][];
    }>;
    __BUZZ_E2E_SET_RELAY_CONNECTION_STATE__?: (state: ConnectionState) => void;
    __BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?: () => ConnectionState;
    __BUZZ_E2E_SET_STALL_WEBSOCKET_SENDS__?: (stall: boolean) => void;
    __BUZZ_E2E_DISCONNECT_MOCK_WEBSOCKETS__?: () => number;
    __BUZZ_E2E_SET_MESH__?: (mesh: {
      admitted?: boolean;
      models?: Array<{ id: string; name: string | null }>;
      denyReason?: string;
    }) => void;
    __BUZZ_E2E_SEED_ACTIVE_TURNS__?: (input: {
      agentPubkey: string;
      channelId: string;
      turnId: string;
      kind?: "turn_started" | "turn_completed";
    }) => void;
    __BUZZ_E2E_SEED_OBSERVER_EVENTS__?: (input: {
      agentPubkey: string;
      events: Array<{
        seq: number;
        timestamp: string;
        kind: string;
        agentIndex: number | null;
        channelId: string | null;
        sessionId: string | null;
        turnId: string | null;
        payload: unknown;
      }>;
    }) => void;
    __BUZZ_E2E_EMIT_MOCK_READ_STATE__?: (input: {
      clientId: string;
      contexts: Record<string, number>;
      createdAt: number;
      slotId: string;
    }) => unknown;
    __BUZZ_E2E_SEED_MOCK_REMINDERS__?: (reminders: RelayEvent[]) => void;
    __BUZZ_E2E_QUERY_CLIENT__?: {
      invalidateQueries: (filters: { queryKey: readonly unknown[] }) => unknown;
    };
  }
}

const DEFAULT_RELAY_HTTP_URL = "http://localhost:3000";
const DEFAULT_RELAY_WS_URL = "ws://localhost:3000";

// NIP event kinds the mock reaction handlers emit.
const KIND_REACTION = 7; // NIP-25 reaction
const KIND_DELETION = 5; // NIP-09 deletion

// Fake media-proxy port the mock answers for `get_media_proxy_port`, so
// `rewriteRelayUrl()` produces a real `http://127.0.0.1:<port>/media/...` src
// in e2e (instead of the `buzz-media://` fallback). The reaction guard
// asserts against this exact port.
const MOCK_MEDIA_PROXY_PORT = 54321;

// A relay-hosted custom emoji used by the reaction guard. Its URL matches
// `rewriteRelayUrl()`'s `/media/{64-hex}.{ext}` pattern on the relay origin, so
// reacting with it exercises the proxy rewrite (unlike the `:buzz:` fixture,
// whose external example.com URL passes through unchanged).
const REACTION_EMOJI_SHORTCODE = "react";
const REACTION_EMOJI_SHA = "c".repeat(64);
const REACTION_EMOJI_URL = `${DEFAULT_RELAY_HTTP_URL}/media/${REACTION_EMOJI_SHA}.png`;

// A reaction-target message seeded into `general` with a real 64-hex event id.
// The reaction guard reacts to THIS message: getReactionTargetId() only accepts
// a 64-hex `e` tag, and the other mock seeds (and user-sent messages) use short
// non-hex ids, so they can't be reaction targets. Content is distinctive so the
// test locates its row without relying on seed ordering.
const REACTION_TARGET_EVENT_ID = "d".repeat(64);
const REACTION_TARGET_CONTENT = "React to me with a custom emoji";
// System-message reaction target id (kind:40099 join event). Distinct 64-hex
// id so it is a valid reaction target and never collides with the regular
// REACTION_TARGET_EVENT_ID.
const SYSTEM_REACTION_TARGET_EVENT_ID = "e".repeat(64);
const E2E_IDENTITY_OVERRIDE_STORAGE_KEY = "buzz:e2e-identity-override.v1";
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
const PROFILE_ONLY_AGENT_PUBKEY =
  "8f83d6b7f3d74f7d933ae3a54dd8c6cc85c7f98e531c16e5a827b953441a8d67";
// A relay-classified bot agent whose declared NIP-OA owner is the mock viewer,
// but which is NOT locally managed. This is the fixture that exercises the
// sidebar's owner-gate path (`viewerIsOwner`), distinct from the local-managed
// path that `mira` (profile-only) and managed-agent fixtures cover.
const OWNED_RELAY_AGENT_PUBKEY =
  "a1b2c3d4e5f60718293a4b5c6d7e8f90112233445566778899aabbccddeeff00";
const MOCK_IDENTITY_PUBKEY = DEFAULT_MOCK_IDENTITY.pubkey;

const mockDisplayNames = new Map<string, string>([
  [MOCK_IDENTITY_PUBKEY, DEFAULT_MOCK_IDENTITY.display_name],
  [ALICE_PUBKEY, "alice"],
  [BOB_PUBKEY, "bob"],
  [CHARLIE_PUBKEY, "charlie"],
  [PROFILE_ONLY_AGENT_PUBKEY, "mira"],
  [OWNED_RELAY_AGENT_PUBKEY, "nadia"],
  [OUTSIDER_PUBKEY, "outsider"],
  [DEFAULT_REAL_IDENTITY.pubkey, DEFAULT_REAL_IDENTITY.username],
]);
const mockAgentPubkeys = new Set([
  ALICE_PUBKEY,
  CHARLIE_PUBKEY,
  PROFILE_ONLY_AGENT_PUBKEY,
  OWNED_RELAY_AGENT_PUBKEY,
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
    member_pubkeys: channel.members.map((member) => member.pubkey),
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
    is_agent: role === "bot" || mockAgentPubkeys.has(pubkey),
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
    avatar_url: agent.avatar_url ?? null,
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

function cloneEngramEntry(entry: RawEngramEntry): RawEngramEntry {
  return {
    ...entry,
    outgoingRefs: [...entry.outgoingRefs],
  };
}

function cloneAgentMemoryListing(
  listing: RawAgentMemoryListing,
): RawAgentMemoryListing {
  return {
    core: listing.core ? cloneEngramEntry(listing.core) : null,
    memories: listing.memories.map(cloneEngramEntry),
    truncated: listing.truncated,
    fetchedAt: listing.fetchedAt,
  };
}

function resetMockRelayMembers(config: E2eConfig | undefined) {
  const pubkey = getMockMemberPubkey(config);
  // Drive the active identity's role from `mock.relayRole` so the e2e harness
  // can exercise the NIP-IA admin gate (owner/admin → true, member/null →
  // false). Default stays `owner` to preserve existing test behavior.
  const role = config?.mock?.relayRole;
  const activeRoleMember =
    role === null
      ? null
      : {
          pubkey,
          role: role ?? "owner",
          added_by: null,
          created_at: isoMinutesAgo(120),
        };
  mockRelayMembers = [
    ...(activeRoleMember ? [activeRoleMember] : []),
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

function buildMockConfigSurface(pubkey: string): {
  runtimeId: string | null;
  runtimeLabel: string | null;
  isPreSpawn: boolean;
  normalized: Record<string, unknown>;
  advanced: unknown[];
  sources: Record<string, unknown>;
} {
  // Goose running — mixed origins, override on model
  const gooseSurface = {
    runtimeId: "goose",
    runtimeLabel: "Goose",
    isPreSpawn: false,
    normalized: {
      model: {
        value: "gpt-4o",
        origin: "buzzExplicit",
        overriddenValue: "gpt-4o-mini",
        overriddenOrigin: "configFile",
        isRequired: false,
        writeVia: { type: "readOnly" },
      },
      provider: {
        value: "openai",
        origin: "configFile",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: {
          type: "gooseNativeConfigWrite",
          configKey: "goose.provider",
        },
      },
      mode: {
        value: "auto",
        origin: "envVar",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "respawnWithEnvVar", envKey: "GOOSE_MODE" },
      },
      thinkingEffort: {
        value: "medium",
        origin: "configFile",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: {
          type: "gooseNativeConfigWrite",
          configKey: "goose.thinkingEffort",
        },
      },
      maxOutputTokens: null,
      contextLimit: null,
      systemPrompt: null,
    },
    advanced: [
      {
        key: "extensions.developer",
        label: "Extension: developer",
        value: "enabled",
        origin: "configFile",
        schemaType: { type: "enum", options: ["enabled", "disabled"] },
        writeVia: {
          type: "gooseNativeConfigWrite",
          configKey: "goose.extensions.developer",
        },
      },
      {
        key: "extensions.web_search",
        label: "Extension: web_search",
        value: "enabled",
        origin: "configFile",
        schemaType: { type: "enum", options: ["enabled", "disabled"] },
        writeVia: {
          type: "gooseNativeConfigWrite",
          configKey: "goose.extensions.web_search",
        },
      },
      {
        key: "extensions.memory",
        label: "Extension: memory",
        value: "disabled",
        origin: "configFile",
        schemaType: { type: "enum", options: ["enabled", "disabled"] },
        writeVia: {
          type: "gooseNativeConfigWrite",
          configKey: "goose.extensions.memory",
        },
      },
    ],
    sources: {
      acpNative: "available",
      acpConfigOptions: "available",
      envVars: "available",
      configFile: "available",
      configFilePath: "~/.config/goose/config.yaml",
    },
  };

  // Claude Code — mostly ACP-sourced
  const claudeSurface = {
    runtimeId: "claude-code",
    runtimeLabel: "Claude Code",
    isPreSpawn: false,
    normalized: {
      model: {
        value: "claude-sonnet-4-20250514",
        origin: "acpConfigOption",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "acpSetConfigOption", configId: "model" },
      },
      provider: {
        value: "anthropic",
        origin: "acpConfigOption",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "acpSetConfigOption", configId: "provider" },
      },
      mode: {
        value: "code",
        origin: "acpConfigOption",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "acpSetConfigOption", configId: "mode" },
      },
      thinkingEffort: {
        value: "high",
        origin: "acpConfigOption",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "acpSetConfigOption", configId: "thinkingEffort" },
      },
      maxOutputTokens: {
        value: "16384",
        origin: "acpConfigOption",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "acpSetConfigOption", configId: "maxOutputTokens" },
      },
      contextLimit: null,
      systemPrompt: null,
    },
    advanced: [],
    sources: {
      acpNative: "available",
      acpConfigOptions: "available",
      envVars: "notApplicable",
      configFile: "available",
      configFilePath: "~/.claude/settings.json",
    },
  };

  // Pre-spawn — model from config file, ACP fields pending
  const preSpawnSurface = {
    runtimeId: "goose",
    runtimeLabel: "Goose",
    isPreSpawn: true,
    normalized: {
      model: {
        value: "gpt-4o-mini",
        origin: "configFile",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "gooseNativeConfigWrite", configKey: "goose.model" },
      },
      provider: {
        value: "openai",
        origin: "configFile",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: {
          type: "gooseNativeConfigWrite",
          configKey: "goose.provider",
        },
      },
      mode: {
        value: null,
        origin: "acpNativeRead",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "readOnly" },
      },
      thinkingEffort: {
        value: null,
        origin: "acpNativeRead",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "readOnly" },
      },
      maxOutputTokens: null,
      contextLimit: null,
      systemPrompt: null,
    },
    advanced: [],
    sources: {
      acpNative: "pending",
      acpConfigOptions: "pending",
      envVars: "available",
      configFile: "available",
      configFilePath: "~/.config/goose/config.yaml",
    },
  };

  // Codex — dual-axis mode
  const codexSurface = {
    runtimeId: "codex",
    runtimeLabel: "Codex",
    isPreSpawn: false,
    normalized: {
      model: {
        value: "codex-mini",
        origin: "configFile",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "gooseNativeConfigWrite", configKey: "goose.model" },
      },
      provider: {
        value: "openai",
        origin: "configFile",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: {
          type: "gooseNativeConfigWrite",
          configKey: "goose.provider",
        },
      },
      mode: {
        value: "suggest / auto-edit",
        origin: "configFile",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "gooseNativeConfigWrite", configKey: "goose.mode" },
      },
      thinkingEffort: null,
      maxOutputTokens: null,
      contextLimit: null,
      systemPrompt: null,
    },
    advanced: [
      {
        key: "approval_policy",
        label: "Approval Policy",
        value: "unless-allow-listed",
        origin: "configFile",
        schemaType: {
          type: "enum",
          options: ["suggest", "auto-edit", "full-auto", "unless-allow-listed"],
        },
        writeVia: {
          type: "gooseNativeConfigWrite",
          configKey: "goose.approval_policy",
        },
      },
      {
        key: "sandbox_mode",
        label: "Sandbox Mode",
        value: "container",
        origin: "envVar",
        schemaType: {
          type: "enum",
          options: ["container", "host", "none"],
        },
        writeVia: { type: "respawnWithEnvVar", envKey: "GOOSE_SANDBOX_MODE" },
      },
    ],
    sources: {
      acpNative: "notApplicable",
      acpConfigOptions: "notApplicable",
      envVars: "available",
      configFile: "available",
      configFilePath: "~/.codex/config.toml",
    },
  };

  // Live runtime override — a persona-linked agent whose session model was
  // switched at runtime. The live model rides over the persona baseline as a
  // secondary value WITHOUT strikethrough (the headline runtimeOverride render).
  const runtimeOverrideSurface = {
    runtimeId: "goose",
    runtimeLabel: "Goose",
    isPreSpawn: false,
    normalized: {
      model: {
        value: "claude-opus-4-20250514",
        origin: "runtimeOverride",
        overriddenValue: "gpt-4o",
        overriddenOrigin: "personaDefault",
        isRequired: false,
        writeVia: { type: "acpSetSessionModel" },
      },
      provider: {
        value: "anthropic",
        origin: "acpConfigOption",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "acpSetConfigOption", configId: "provider" },
      },
      mode: {
        value: "auto",
        origin: "envVar",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "respawnWithEnvVar", envKey: "GOOSE_MODE" },
      },
      thinkingEffort: {
        value: "high",
        origin: "configFile",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: {
          type: "gooseNativeConfigWrite",
          configKey: "goose.thinkingEffort",
        },
      },
      maxOutputTokens: null,
      contextLimit: null,
      systemPrompt: null,
    },
    advanced: [],
    sources: {
      acpNative: "available",
      acpConfigOptions: "available",
      envVars: "available",
      configFile: "available",
      configFilePath: "~/.config/goose/config.yaml",
    },
  };

  // Mixed-provenance showcase — top-level rows carry different origins so the
  // panel witnesses distinct provenance labels in one frame: "Set in Buzz",
  // "Inherited from persona", "From config file (...)" and
  // "From environment variable (...)".
  const multiOriginSurface = {
    runtimeId: "goose",
    runtimeLabel: "Goose",
    isPreSpawn: false,
    normalized: {
      model: {
        value: "gpt-4o",
        origin: "buzzExplicit",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "readOnly" },
      },
      provider: {
        value: "openai",
        origin: "personaDefault",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "readOnly" },
      },
      mode: {
        value: "auto",
        origin: "envVar",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: { type: "respawnWithEnvVar", envKey: "GOOSE_MODE" },
      },
      thinkingEffort: {
        value: "medium",
        origin: "configFile",
        overriddenValue: null,
        overriddenOrigin: null,
        isRequired: false,
        writeVia: {
          type: "gooseNativeConfigWrite",
          configKey: "goose.thinkingEffort",
        },
      },
      maxOutputTokens: null,
      contextLimit: null,
      systemPrompt: null,
    },
    advanced: [],
    sources: {
      acpNative: "available",
      acpConfigOptions: "available",
      envVars: "available",
      configFile: "available",
      configFilePath: "~/.config/goose/config.yaml",
    },
  };

  // Map well-known test pubkeys to specific fixtures
  // Synthetic agent for the multi-origin provenance showcase (not a TEST_IDENTITY).
  const PUBKEY_MULTI_ORIGIN =
    "abc1230000000000000000000000000000000000000000000000000000000def";

  switch (pubkey) {
    case ALICE_PUBKEY:
      return claudeSurface;
    case BOB_PUBKEY:
      return preSpawnSurface;
    case CHARLIE_PUBKEY:
      return codexSurface;
    case OUTSIDER_PUBKEY:
      return runtimeOverrideSurface;
    case PUBKEY_MULTI_ORIGIN:
      return multiOriginSurface;
    default:
      return gooseSurface;
  }
}

function buildSeededManagedAgent(seed: MockManagedAgentSeed): MockManagedAgent {
  const now = new Date().toISOString();
  const status = seed.status ?? "stopped";

  return {
    pubkey: seed.pubkey,
    name: seed.name,
    persona_id: seed.personaId ?? null,
    relay_url: DEFAULT_RELAY_WS_URL,
    acp_command: "buzz-acp",
    agent_command: "goose",
    agent_args: ["acp"],
    mcp_command: "",
    turn_timeout_seconds: 320,
    idle_timeout_seconds: null,
    max_turn_duration_seconds: null,
    parallelism: 1,
    system_prompt: null,
    avatar_url: seed.avatarUrl ?? null,
    model: null,
    env_vars: {},
    status,
    pid: status === "running" ? 42000 + mockManagedAgents.length : null,
    created_at: now,
    updated_at: now,
    last_started_at: status === "running" ? now : null,
    last_stopped_at: status === "stopped" ? now : null,
    last_exit_code: null,
    last_error: seed.lastError ?? null,
    log_path: `/tmp/mock-agent-${seed.pubkey}.log`,
    start_on_app_launch: true,
    backend: seed.backend ?? { type: "local" },
    backend_agent_id: null,
    respond_to: seed.respondTo ?? "owner-only",
    respond_to_allowlist: seed.respondToAllowlist ?? [],
    private_key_nsec: `nsec1mock${seed.pubkey.slice(0, 20)}`,
    log_lines: [
      `buzz-acp starting: relay=${DEFAULT_RELAY_WS_URL} agent_pubkey=${seed.pubkey} parallelism=1`,
      "profile created; harness not started",
    ],
  };
}

function resetMockRelayAgents(config?: E2eConfig) {
  mockRelayAgents = defaultMockRelayAgents.map((agent) => ({
    ...agent,
    channels: [...agent.channels],
    channel_ids: [...agent.channel_ids],
    capabilities: [...agent.capabilities],
    respond_to_allowlist: [...(agent.respond_to_allowlist ?? [])],
  }));

  for (const seed of config?.mock?.relayAgents ?? []) {
    const channels = mockChannels.filter((channel) => {
      return (
        seed.channelIds?.includes(channel.id) ||
        seed.channelNames?.includes(channel.name)
      );
    });
    mockRelayAgents.push({
      pubkey: seed.pubkey,
      name: seed.name,
      agent_type: seed.agentType ?? "goose",
      channels: channels.map((channel) => channel.name),
      channel_ids: channels.map((channel) => channel.id),
      capabilities: seed.capabilities ?? ["messages", "channels", "mcp"],
      status: seed.status ?? "online",
      respond_to: seed.respondTo ?? "owner-only",
      respond_to_allowlist: seed.respondToAllowlist ?? [],
    });
  }
}

function resetMockManagedAgents(config?: E2eConfig) {
  mockManagedAgents = [];

  for (const seed of config?.mock?.managedAgents ?? []) {
    mockManagedAgents.push(buildSeededManagedAgent(seed));
    applyMockDisplayName(seed.pubkey, seed.name);
    mockAgentPubkeys.add(seed.pubkey);
    mockProfiles.set(seed.pubkey, {
      pubkey: seed.pubkey,
      display_name: seed.name,
      avatar_url: null,
      about: null,
      nip05_handle: null,
      owner_pubkey: MOCK_IDENTITY_PUBKEY,
      is_agent: true,
    });
    for (const channel of mockChannels) {
      const isSeedChannel =
        seed.channelIds?.includes(channel.id) ||
        seed.channelNames?.includes(channel.name);
      if (
        !isSeedChannel ||
        channel.members.some((member) => member.pubkey === seed.pubkey)
      ) {
        continue;
      }

      channel.members.push({
        pubkey: seed.pubkey,
        role: "bot",
        is_agent: true,
        joined_at: new Date().toISOString(),
        display_name: seed.name,
      });
      syncMockChannel(channel);
      touchMockChannel(channel);
    }
  }

  syncMockRelayAgentsFromManagedAgents();
}

const BUILT_IN_PERSONA_AVATAR_URLS = {
  productStrategist:
    "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxMjggMTI4Ij48cmVjdCB3aWR0aD0iMTI4IiBoZWlnaHQ9IjEyOCIgcng9IjMyIiBmaWxsPSIjMmY2ZjczIi8+PGNpcmNsZSBjeD0iOTYiIGN5PSIzMiIgcj0iMTYiIGZpbGw9IiNkZmY3ZjQiIGZpbGwtb3BhY2l0eT0iLjI4Ii8+PHBhdGggZD0iTTI0IDk0YzE0LTI1IDI3LTM3IDQwLTM3czI2IDEyIDQwIDM3IiBmaWxsPSJub25lIiBzdHJva2U9IiNkZmY3ZjQiIHN0cm9rZS13aWR0aD0iOCIgc3Ryb2tlLWxpbmVjYXA9InJvdW5kIi8+PHRleHQgeD0iNjQiIHk9IjcyIiB0ZXh0LWFuY2hvcj0ibWlkZGxlIiBmb250LWZhbWlseT0iSW50ZXIsQXJpYWwsc2Fucy1zZXJpZiIgZm9udC1zaXplPSIzNCIgZm9udC13ZWlnaHQ9IjcwMCIgZmlsbD0iI2RmZjdmNCI+UFM8L3RleHQ+PC9zdmc+",
  implementationPartner:
    "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxMjggMTI4Ij48cmVjdCB3aWR0aD0iMTI4IiBoZWlnaHQ9IjEyOCIgcng9IjMyIiBmaWxsPSIjNmY0ZTlhIi8+PGNpcmNsZSBjeD0iOTYiIGN5PSIzMiIgcj0iMTYiIGZpbGw9IiNmMGU4ZmYiIGZpbGwtb3BhY2l0eT0iLjI4Ii8+PHBhdGggZD0iTTI0IDk0YzE0LTI1IDI3LTM3IDQwLTM3czI2IDEyIDQwIDM3IiBmaWxsPSJub25lIiBzdHJva2U9IiNmMGU4ZmYiIHN0cm9rZS13aWR0aD0iOCIgc3Ryb2tlLWxpbmVjYXA9InJvdW5kIi8+PHRleHQgeD0iNjQiIHk9IjcyIiB0ZXh0LWFuY2hvcj0ibWlkZGxlIiBmb250LWZhbWlseT0iSW50ZXIsQXJpYWwsc2Fucy1zZXJpZiIgZm9udC1zaXplPSIzNCIgZm9udC13ZWlnaHQ9IjcwMCIgZmlsbD0iI2YwZThmZiI+SVA8L3RleHQ+PC9zdmc+",
  qaReviewer:
    "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxMjggMTI4Ij48cmVjdCB3aWR0aD0iMTI4IiBoZWlnaHQ9IjEyOCIgcng9IjMyIiBmaWxsPSIjOWE1YTFmIi8+PGNpcmNsZSBjeD0iOTYiIGN5PSIzMiIgcj0iMTYiIGZpbGw9IiNmZmYwZGMiIGZpbGwtb3BhY2l0eT0iLjI4Ii8+PHBhdGggZD0iTTI0IDk0YzE0LTI1IDI3LTM3IDQwLTM3czI2IDEyIDQwIDM3IiBmaWxsPSJub25lIiBzdHJva2U9IiNmZmYwZGMiIHN0cm9rZS13aWR0aD0iOCIgc3Ryb2tlLWxpbmVjYXA9InJvdW5kIi8+PHRleHQgeD0iNjQiIHk9IjcyIiB0ZXh0LWFuY2hvcj0ibWlkZGxlIiBmb250LWZhbWlseT0iSW50ZXIsQXJpYWwsc2Fucy1zZXJpZiIgZm9udC1zaXplPSIzNCIgZm9udC13ZWlnaHQ9IjcwMCIgZmlsbD0iI2ZmZjBkYyI+UUE8L3RleHQ+PC9zdmc+",
  workCoordinator:
    "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxMjggMTI4Ij48cmVjdCB3aWR0aD0iMTI4IiBoZWlnaHQ9IjEyOCIgcng9IjMyIiBmaWxsPSIjMzQ1ZjhjIi8+PGNpcmNsZSBjeD0iOTYiIGN5PSIzMiIgcj0iMTYiIGZpbGw9IiNlM2YwZmYiIGZpbGwtb3BhY2l0eT0iLjI4Ii8+PHBhdGggZD0iTTI0IDk0YzE0LTI1IDI3LTM3IDQwLTM3czI2IDEyIDQwIDM3IiBmaWxsPSJub25lIiBzdHJva2U9IiNlM2YwZmYiIHN0cm9rZS13aWR0aD0iOCIgc3Ryb2tlLWxpbmVjYXA9InJvdW5kIi8+PHRleHQgeD0iNjQiIHk9IjcyIiB0ZXh0LWFuY2hvcj0ibWlkZGxlIiBmb250LWZhbWlseT0iSW50ZXIsQXJpYWwsc2Fucy1zZXJpZiIgZm9udC1zaXplPSIzNCIgZm9udC13ZWlnaHQ9IjcwMCIgZmlsbD0iI2UzZjBmZiI+V0M8L3RleHQ+PC9zdmc+",
  supportGuide:
    "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxMjggMTI4Ij48cmVjdCB3aWR0aD0iMTI4IiBoZWlnaHQ9IjEyOCIgcng9IjMyIiBmaWxsPSIjNmE3MDMxIi8+PGNpcmNsZSBjeD0iOTYiIGN5PSIzMiIgcj0iMTYiIGZpbGw9IiNmMmY2ZDciIGZpbGwtb3BhY2l0eT0iLjI4Ii8+PHBhdGggZD0iTTI0IDk0YzE0LTI1IDI3LTM3IDQwLTM3czI2IDEyIDQwIDM3IiBmaWxsPSJub25lIiBzdHJva2U9IiNmMmY2ZDciIHN0cm9rZS13aWR0aD0iOCIgc3Ryb2tlLWxpbmVjYXA9InJvdW5kIi8+PHRleHQgeD0iNjQiIHk9IjcyIiB0ZXh0LWFuY2hvcj0ibWlkZGxlIiBmb250LWZhbWlseT0iSW50ZXIsQXJpYWwsc2Fucy1zZXJpZiIgZm9udC1zaXplPSIzNCIgZm9udC13ZWlnaHQ9IjcwMCIgZmlsbD0iI2YyZjZkNyI+U0c8L3RleHQ+PC9zdmc+",
  experimentDesigner:
    "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxMjggMTI4Ij48cmVjdCB3aWR0aD0iMTI4IiBoZWlnaHQ9IjEyOCIgcng9IjMyIiBmaWxsPSIjOGIzZjVlIi8+PGNpcmNsZSBjeD0iOTYiIGN5PSIzMiIgcj0iMTYiIGZpbGw9IiNmZmU3ZjAiIGZpbGwtb3BhY2l0eT0iLjI4Ii8+PHBhdGggZD0iTTI0IDk0YzE0LTI1IDI3LTM3IDQwLTM3czI2IDEyIDQwIDM3IiBmaWxsPSJub25lIiBzdHJva2U9IiNmZmU3ZjAiIHN0cm9rZS13aWR0aD0iOCIgc3Ryb2tlLWxpbmVjYXA9InJvdW5kIi8+PHRleHQgeD0iNjQiIHk9IjcyIiB0ZXh0LWFuY2hvcj0ibWlkZGxlIiBmb250LWZhbWlseT0iSW50ZXIsQXJpYWwsc2Fucy1zZXJpZiIgZm9udC1zaXplPSIzNCIgZm9udC13ZWlnaHQ9IjcwMCIgZmlsbD0iI2ZmZTdmMCI+RUQ8L3RleHQ+PC9zdmc+",
} as const;

function resetMockPersonas(config?: E2eConfig) {
  const now = new Date().toISOString();
  const activePersonaIds = new Set(config?.mock?.activePersonaIds ?? []);
  const builtInPersonas = [
    {
      id: "builtin:product-strategist",
      display_name: "Product Strategist",
      avatar_url: BUILT_IN_PERSONA_AVATAR_URLS.productStrategist,
      system_prompt:
        "You are a product strategy agent. You help turn broad ideas into clear product and design direction.\n\n# Focus\n\nClarify the goal, identify the audience, and call out the tradeoffs that matter.",
    },
    {
      id: "builtin:implementation-partner",
      display_name: "Implementation Partner",
      avatar_url: BUILT_IN_PERSONA_AVATAR_URLS.implementationPartner,
      system_prompt:
        "You are an implementation partner agent. You help turn scoped plans into working changes.\n\n# Focus\n\nPrefer small, direct edits that follow the existing codebase.",
    },
    {
      id: "builtin:qa-reviewer",
      display_name: "QA Reviewer",
      avatar_url: BUILT_IN_PERSONA_AVATAR_URLS.qaReviewer,
      system_prompt:
        "You are a QA reviewer agent. You look for the ways a change might break.\n\n# Focus\n\nInspect state transitions, empty states, permissions, accessibility, and failure paths.",
    },
    {
      id: "builtin:fizz",
      display_name: "Fizz",
      avatar_url: null,
      system_prompt: "You are Fizz.",
    },
    {
      id: "builtin:work-coordinator",
      display_name: "Work Coordinator",
      avatar_url: BUILT_IN_PERSONA_AVATAR_URLS.workCoordinator,
      system_prompt:
        "You are a work coordination agent. You keep multi-step work organized and grounded.\n\n# Focus\n\nTrack goals, dependencies, and follow-ups.",
    },
    {
      id: "builtin:support-guide",
      display_name: "Support Guide",
      avatar_url: BUILT_IN_PERSONA_AVATAR_URLS.supportGuide,
      system_prompt:
        "You are a support and onboarding agent. You help people feel oriented quickly.\n\n# Focus\n\nExplain unfamiliar flows plainly, surface the next useful step, and keep guidance reassuring without becoming verbose.",
    },
    {
      id: "builtin:experiment-designer",
      display_name: "Experiment Designer",
      avatar_url: BUILT_IN_PERSONA_AVATAR_URLS.experimentDesigner,
      system_prompt:
        "You are an experiment design agent. You help turn uncertainty into experiments.\n\n# Focus\n\nBreak big questions into small trials, name what would prove or disprove an idea, and keep momentum through ambiguity.",
    },
  ];
  mockPersonas = builtInPersonas.map((persona) => ({
    id: persona.id,
    display_name: persona.display_name,
    avatar_url: persona.avatar_url,
    system_prompt: persona.system_prompt,
    runtime: persona.id === "builtin:fizz" ? "goose" : null,
    model: null,
    provider: null,
    name_pool: [],
    is_builtin: true,
    is_active: activePersonaIds.has(persona.id),
    source_team: null,
    created_at: now,
    updated_at: now,
  }));

  for (const persona of config?.mock?.personas ?? []) {
    mockPersonas.push({
      id: persona.id ?? crypto.randomUUID(),
      display_name: persona.displayName,
      avatar_url: persona.avatarUrl ?? null,
      system_prompt: persona.systemPrompt,
      is_builtin: false,
      is_active: persona.isActive ?? true,
      source_team: persona.sourceTeam ?? null,
      env_vars: { ...(persona.envVars ?? {}) },
      created_at: now,
      updated_at: now,
    });
  }
}

function resetMockTeams() {
  const now = new Date().toISOString();
  mockTeams = [
    {
      id: "team-engineering-001",
      name: "Engineering",
      description: "Core engineering personas",
      persona_ids: [],
      is_builtin: false,
      source_dir: null,
      is_symlink: false,
      symlink_target: null,
      version: null,
      created_at: now,
      updated_at: now,
    },
    {
      id: "team-research-002",
      name: "Research Agents",
      description: "Directory-backed research team",
      persona_ids: [],
      is_builtin: false,
      source_dir: "/Users/dev/agents/research",
      is_symlink: false,
      symlink_target: null,
      version: "1.2.0",
      created_at: now,
      updated_at: now,
    },
    {
      id: "team-platform-003",
      name: "Platform Tools",
      description: "Symlinked platform team",
      persona_ids: [],
      is_builtin: false,
      source_dir: "/Users/dev/agents/platform",
      is_symlink: true,
      symlink_target: "/opt/shared-teams/platform",
      version: "2.0.1",
      created_at: now,
      updated_at: now,
    },
  ];
}

function seedMockSearchProfiles(config?: E2eConfig) {
  for (const seed of config?.mock?.searchProfiles ?? []) {
    const pubkey = seed.pubkey.toLowerCase();
    const profile = {
      pubkey,
      display_name: seed.displayName,
      avatar_url: seed.avatarUrl ?? null,
      about: seed.about ?? null,
      nip05_handle: seed.nip05Handle ?? null,
      owner_pubkey: seed.ownerPubkey ?? null,
      is_agent: seed.isAgent ?? false,
    };
    mockProfiles.set(pubkey, profile);
    applyMockDisplayName(pubkey, seed.displayName);
    if (seed.isAgent) {
      mockAgentPubkeys.add(pubkey);
    }
  }
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
    owner_pubkey: null,
    is_agent: mockAgentPubkeys.has(normalizedPubkey),
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
      createMockMember(PROFILE_ONLY_AGENT_PUBKEY, "member", 840),
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
  // Reproduces the all-replies-window regression (NIP-RS Fix A): a busy
  // single-thread channel whose top-level root has scrolled past the history
  // limit. `last_message_at` is a far-future timestamp standing in for the
  // backend's reply-inclusive MAX(created_at) — it is NEWER than any top-level
  // message the window can load, so falling back to it would advance the
  // channel marker past unread replies. Seeded as its own channel so existing
  // channels' unread state is undisturbed.
  createMockChannel({
    id: "fa11bac0-0000-4000-8000-000000000012",
    name: "all-replies",
    channel_type: "stream",
    visibility: "open",
    description: "Single-thread channel with the root past the history limit",
    topic: null,
    purpose: null,
    last_message_at: new Date("2999-01-01T00:00:00.000Z").toISOString(),
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
      createMockMember(OWNED_RELAY_AGENT_PUBKEY, "member", 600),
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
    id: "3c2d9f0a-1b44-5e77-9a21-6f8b0c4d2e91",
    name: "secret-projects",
    channel_type: "stream",
    visibility: "private",
    description: "Private project room",
    topic: "Skunkworks",
    purpose: "Coordinate confidential project work.",
    last_message_at: null,
    archived_at: null,
    created_by: ALICE_PUBKEY,
    topic_set_by: ALICE_PUBKEY,
    topic_set_at: isoMinutesAgo(120),
    purpose_set_by: ALICE_PUBKEY,
    purpose_set_at: isoMinutesAgo(130),
    topic_required: false,
    max_members: null,
    nip29_group_id: null,
    created_minutes_ago: 600,
    updated_minutes_ago: 120,
    members: [
      createMockMember(ALICE_PUBKEY, "owner", 600),
      createMockMember(MOCK_IDENTITY_PUBKEY, "member", 540),
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
  // Deep history channel for the load-older-under-virtualization E2E. Seeded
  // with more messages than CHANNEL_HISTORY_LIMIT (300) so the initial load
  // windows to the newest page and a `fetchOlder` (until-cursor) prepend has
  // genuinely older rows to add — exercising the scroll-restore anchor under
  // virtualization. Its own channel so existing channels' row-index and unread
  // assertions stay undisturbed.
  createMockChannel({
    id: "feedf00d-0000-4000-8000-000000000007",
    name: "deep-history",
    channel_type: "stream",
    visibility: "open",
    description: "Channel with paginated history for load-older tests",
    topic: null,
    purpose: null,
    last_message_at: isoMinutesAgo(1),
    archived_at: null,
    created_by: ALICE_PUBKEY,
    topic_set_by: null,
    topic_set_at: null,
    purpose_set_by: null,
    purpose_set_at: null,
    topic_required: false,
    max_members: null,
    nip29_group_id: null,
    created_minutes_ago: 2000,
    updated_minutes_ago: 1,
    members: [
      createMockMember(ALICE_PUBKEY, "owner", 2000),
      createMockMember(MOCK_IDENTITY_PUBKEY, "member", 1900),
    ],
  }),
];

const mockMessages = new Map<string, RelayEvent[]>();
const mockUserStatuses: RelayEvent[] = [];
const mockReminderEvents: RelayEvent[] = [];
let mockRelayMembers: RawRelayMember[] = [];
const mockSockets = new Map<number, MockSocket>();
let mockWebsocketSendMutexWedged = false;
const realSockets = new Map<number, WebSocket>();
let mockManagedAgents: MockManagedAgent[] = [];

// Mesh-compute mock state — TEST-ONLY.
//
// This entire module (e2eBridge.ts) is loaded only when `window.__BUZZ_E2E__`
// is set by the Playwright harness; it never runs in a shipped build. These
// handlers stub the `mesh_*` Tauri commands with the SHAPES the UI expects
// (availability, node status, preset) so the desktop UI flow can be exercised
// in a browser. They deliberately do NOT model real admission, real inference,
// or real mesh routing — those are proven by the Rust layer-2 tests and the
// on-hardware layer-1 example. Do not port any of this into production code.
const mockMeshState: {
  admitted: boolean;
  models: Array<{ id: string; name: string | null }>;
  denyReason: string;
  nodeState: "off" | "running";
  nodeMode: "serve" | "client" | null;
} = {
  admitted: true,
  models: [
    { id: "hf://demo/SmolLM2-135M-Instruct-GGUF:Q4_K_M", name: "SmolLM2 135M" },
  ],
  denyReason: "not a relay member",
  nodeState: "off",
  nodeMode: null,
};

function resetMockMesh() {
  mockMeshState.admitted = true;
  mockMeshState.models = [
    { id: "hf://demo/SmolLM2-135M-Instruct-GGUF:Q4_K_M", name: "SmolLM2 135M" },
  ];
  mockMeshState.denyReason = "not a relay member";
  mockMeshState.nodeState = "off";
  mockMeshState.nodeMode = null;
}
let mockPersonas: RawPersona[] = [];
let mockTeams: RawTeam[] = [];
// Listeners registered via the mock __TAURI_INTERNALS__.listen — keyed by event name.
const tauriEventListeners = new Map<string, Set<() => void>>();
const defaultMockRelayAgents: RawRelayAgent[] = [
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
    respond_to: "anyone",
    respond_to_allowlist: [],
  },
  {
    pubkey: CHARLIE_PUBKEY,
    name: "charlie",
    agent_type: "codex",
    channels: ["general"],
    channel_ids: ["9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50"],
    capabilities: ["code", "reviews"],
    status: "away",
    respond_to: "anyone",
    respond_to_allowlist: [],
  },
];
let mockRelayAgents: RawRelayAgent[] = defaultMockRelayAgents.map((agent) => ({
  ...agent,
  channels: [...agent.channels],
  channel_ids: [...agent.channel_ids],
  capabilities: [...agent.capabilities],
  respond_to_allowlist: [...(agent.respond_to_allowlist ?? [])],
}));

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
      owner_pubkey: null,
      is_agent: false,
    },
  ],
  [
    PROFILE_ONLY_AGENT_PUBKEY,
    {
      pubkey: PROFILE_ONLY_AGENT_PUBKEY,
      display_name: "mira",
      avatar_url: null,
      about: null,
      nip05_handle: null,
      owner_pubkey: MOCK_IDENTITY_PUBKEY,
      is_agent: true,
    },
  ],
  [
    OWNED_RELAY_AGENT_PUBKEY,
    {
      pubkey: OWNED_RELAY_AGENT_PUBKEY,
      display_name: "nadia",
      avatar_url: null,
      about: null,
      nip05_handle: null,
      owner_pubkey: MOCK_IDENTITY_PUBKEY,
      is_agent: true,
    },
  ],
]);
const mockPresence = new Map<string, PresenceStatus>([
  [MOCK_IDENTITY_PUBKEY, "offline"],
  [DEFAULT_REAL_IDENTITY.pubkey, "offline"],
  [ALICE_PUBKEY, "online"],
  [BOB_PUBKEY, "away"],
  [CHARLIE_PUBKEY, "online"],
  [PROFILE_ONLY_AGENT_PUBKEY, "online"],
  [OWNED_RELAY_AGENT_PUBKEY, "online"],
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
        respond_to: agent.respond_to,
        respond_to_allowlist: [...agent.respond_to_allowlist],
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
  return window.__BUZZ_E2E__;
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

function writeStoredIdentityOverride(identity: TestIdentity) {
  window.localStorage.setItem(
    E2E_IDENTITY_OVERRIDE_STORAGE_KEY,
    JSON.stringify(identity),
  );
}

function importMockIdentity(nsec: string) {
  const decoded = decode(nsec.trim());
  if (decoded.type !== "nsec") {
    throw new Error("Invalid Nostr private key.");
  }

  const privateKey = bytesToHex(decoded.data);
  const pubkey = getPublicKey(decoded.data);
  const username = mockDisplayNames.get(pubkey) ?? "";
  const identity = {
    privateKey,
    pubkey,
    username,
  };
  writeStoredIdentityOverride(identity);
  if (!mockProfiles.has(pubkey)) {
    mockProfiles.set(pubkey, {
      pubkey,
      display_name: username || null,
      avatar_url: null,
      about: null,
      nip05_handle: null,
      owner_pubkey: null,
    });
  }

  return {
    pubkey,
    display_name: username,
  };
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
    owner_pubkey: null,
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
            created_at: Math.floor(Date.now() / 1000) - 120,
            kind: 9,
            tags: [["h", channelId]],
            content: "Welcome to #general",
            sig: "mocksig".repeat(20).slice(0, 128),
          },
          // Alice authored — gives e2e specs a non-self profile pane to open
          // by clicking the second message-row's author button. Used by
          // tests/e2e/identity-archive.spec.ts to exercise the admin / OA /
          // none-of-the-above branches of the NIP-IA gate. Both seeds are
          // backdated (welcome at -120s, Alice at -60s) so user-sent messages
          // in other specs always land after both — preserving
          // `message-row.first()` = welcome and `.last()` = sent.
          {
            id: "mock-general-alice",
            pubkey: ALICE_PUBKEY,
            created_at: Math.floor(Date.now() / 1000) - 60,
            kind: 9,
            tags: [["h", channelId]],
            content: "Hey team — checking in.",
            sig: "mocksig".repeat(20).slice(0, 128),
          },
          // Reaction-target seed for the custom-emoji reaction guard. Real
          // 64-hex id so getReactionTargetId() accepts it as a reaction target
          // (the short-id seeds above can't be reacted to). Backdated after the
          // other seeds, so it stays at row index >= 2 and never displaces
          // first()=welcome / nth(1)=alice that other specs rely on.
          {
            id: REACTION_TARGET_EVENT_ID,
            pubkey: ALICE_PUBKEY,
            created_at: Math.floor(Date.now() / 1000) - 45,
            kind: 9,
            tags: [["h", channelId]],
            content: REACTION_TARGET_CONTENT,
            sig: "mocksig".repeat(20).slice(0, 128),
          },
          // System-message reaction target. A kind:40099 join event renders via
          // SystemMessageRow (testid `system-message-row`, NOT `message-row`),
          // so it never displaces the `message-row` index assertions other
          // specs rely on. Real 64-hex id so getReactionTargetId() accepts it
          // as a reaction target — this is the surface the original "react to a
          // system message" bug lived on. Backdated like the other seeds.
          {
            id: SYSTEM_REACTION_TARGET_EVENT_ID,
            pubkey: ALICE_PUBKEY,
            created_at: Math.floor(Date.now() / 1000) - 30,
            kind: KIND_SYSTEM_MESSAGE,
            tags: [["h", channelId]],
            content: JSON.stringify({
              type: "member_joined",
              actor: ALICE_PUBKEY,
              target: ALICE_PUBKEY,
            }),
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
            // Filler replies so the thread overflows the panel viewport — the
            // deep-link target (mock-forum-release-deeplink) sits below the fold
            // at open, proving scrollIntoView lands an offscreen content-
            // visibility row. Named IDs above are untouched.
            ...Array.from({ length: 24 }, (_, index) => ({
              id:
                index === 23
                  ? "mock-forum-release-deeplink"
                  : `mock-forum-release-filler-${index}`,
              pubkey: ALICE_PUBKEY,
              created_at: Math.floor(Date.now() / 1000) - (79 - index) * 60,
              kind: 45003,
              tags: buildReplyMessageTags(
                channelId,
                ALICE_PUBKEY,
                "mock-forum-release-thread",
                "mock-forum-release-thread",
                undefined,
              ),
              content:
                index === 23
                  ? "Deep-link target: confirmed the rollout plan end to end."
                  : `Follow-up note #${index + 1} on the release checklist.`,
              sig: "mocksig".repeat(20).slice(0, 128),
            })),
          ]
        : channelId === "94a444a4-c0a3-5966-ab05-530c6ddc2301"
          ? [
              // Charlie is a `bot` member of #agents (see channel seed), so this
              // message renders with role="bot" — the surface whose avatar opens
              // a managed-agent profile panel / hover popover with active-turn
              // badges. #agents has no message-row index assertions, so seeding
              // here is safe for existing specs.
              {
                id: "mock-agents-charlie",
                pubkey: CHARLIE_PUBKEY,
                created_at: Math.floor(Date.now() / 1000) - 90,
                kind: 9,
                tags: [["h", channelId]],
                content: "Indexing the channel catalog now.",
                sig: "mocksig".repeat(20).slice(0, 128),
              },
              // Owned remote relay agent: declared-owned by the mock viewer,
              // present in the relay registry, but NOT locally managed. This
              // keeps the profile Runtime-tab owner gate honest.
              {
                id: "mock-agents-owned-relay-nadia",
                pubkey: OWNED_RELAY_AGENT_PUBKEY,
                created_at: Math.floor(Date.now() / 1000) - 85,
                kind: 9,
                tags: [["h", channelId]],
                content: "Indexing remotely for my owner.",
                sig: "mocksig".repeat(20).slice(0, 128),
              },
              // Seed one message per managed agent that is a member of #agents.
              // This lets e2e specs open the profile panel by clicking the
              // agent's avatar in a message-row (the same pattern as charlie).
              ...mockManagedAgents
                .filter((agent) =>
                  mockChannels
                    .find((ch) => ch.id === channelId)
                    ?.members.some((m) => m.pubkey === agent.pubkey),
                )
                .map((agent, index) => ({
                  id: `mock-agents-managed-${agent.pubkey.slice(0, 8)}`,
                  pubkey: agent.pubkey,
                  created_at: Math.floor(Date.now() / 1000) - 80 + index,
                  kind: 9 as const,
                  tags: [["h", channelId]],
                  content: `${agent.name} reporting in.`,
                  sig: "mocksig".repeat(20).slice(0, 128),
                })),
            ]
          : channelId === "feedf00d-0000-4000-8000-000000000007"
            ? // 600 messages > CHANNEL_HISTORY_LIMIT (300): the initial load
              // windows to the newest 300, leaving 300 older behind the until
              // cursor — enough for several full fetchOlder pages (batch 100),
              // so the load-older anchor restore is exercised across REPEATED
              // prepend cycles, not a single lucky pass. created_at increases
              // with index (oldest first) so message N+1 is newer than N — the
              // anchor restores the first-visible row across each prepend.
              Array.from({ length: 600 }, (_, index) => ({
                id: `mock-deep-history-${index}`,
                pubkey: index % 2 === 0 ? ALICE_PUBKEY : MOCK_IDENTITY_PUBKEY,
                created_at: Math.floor(Date.now() / 1000) - (600 - index) * 60,
                kind: 9,
                tags: [["h", channelId]],
                content: `Deep history message #${index}`,
                sig: "mocksig".repeat(20).slice(0, 128),
              }))
            : [];

  mockMessages.set(channelId, seeded);
  return seeded;
}

function prependMockHistory(input: {
  channelName: string;
  count: number;
  startIndex?: number;
  lineCount?: number;
  createdAtStart?: number;
  emit?: boolean;
}) {
  const channel = mockChannels.find(
    (candidate) => candidate.name === input.channelName,
  );
  if (!channel) {
    throw new Error(`Unknown mock channel: ${input.channelName}`);
  }

  const store = getMockMessageStore(channel.id);
  const earliestCreatedAt = store.reduce(
    (earliest, event) => Math.min(earliest, event.created_at),
    Math.floor(Date.now() / 1000),
  );
  const createdAtStart =
    input.createdAtStart ?? earliestCreatedAt - input.count - 1;
  const startIndex = input.startIndex ?? 0;
  const lineCount = input.lineCount ?? 1;

  const events = Array.from({ length: input.count }, (_, offset) => {
    const index = startIndex + offset;
    const body = Array.from(
      { length: lineCount },
      (_unused, lineIndex) => `mock older ${index} line ${lineIndex + 1}`,
    ).join("\n");

    return createMockEvent(
      9,
      body,
      [["h", channel.id]],
      ALICE_PUBKEY,
      createdAtStart + offset,
      `mock-older-${channel.name}-${index}`.replace(/[^a-zA-Z0-9]/g, ""),
    );
  });

  store.unshift(...events);
  store.sort((left, right) => left.created_at - right.created_at);

  if (input.emit) {
    for (const event of events) {
      emitMockLiveEvent(channel.id, event);
    }
  }

  return events;
}

function emitMockHistory(
  socket: MockSocket,
  subId: string,
  channelId: string,
  filter: MockFilter,
) {
  const events = getMockMessageStore(channelId)
    .filter((event) => {
      if (filter.kinds && !filter.kinds.includes(event.kind)) {
        return false;
      }
      if (filter.since !== undefined && event.created_at < filter.since) {
        return false;
      }
      if (filter.until !== undefined && event.created_at > filter.until) {
        return false;
      }
      return true;
    })
    // Relay order is `created_at DESC, id ASC` — match it (both the WS history
    // page and the `get_channel_messages_before` keyset are backed by that one
    // order in production, so the mock must be self-consistent too, else a
    // same-second slice returned here won't line up with the keyset's tiebreak
    // and the dense-second escape hatch can't prove completeness). Bare `until`
    // still can't advance past a second denser than one page; the composite
    // keyset is the escape hatch.
    .sort(
      (left, right) =>
        right.created_at - left.created_at || left.id.localeCompare(right.id),
    )
    .slice(0, filter.limit ?? 50)
    .sort(
      (left, right) =>
        left.created_at - right.created_at || left.id.localeCompare(right.id),
    );

  const emit = () => {
    for (const event of events) {
      sendWsText(socket.handler, ["EVENT", subId, event]);
    }
    sendWsText(socket.handler, ["EOSE", subId]);
  };

  // Optionally pace older-history fetches so e2e tests can observe the
  // in-flight prepend window (scroll up, abandon, etc.). Scoped to
  // `history-` subscriptions — the prefix `relayClientSession` uses for
  // older-message pagination — so live/initial subscriptions stay instant.
  const delayMs = getConfig()?.mock?.historyDelayMs ?? 0;
  const isVisibleOlderHistoryPage =
    subId.startsWith("history-") && filter.until !== undefined && !filter["#e"];
  if (isVisibleOlderHistoryPage) {
    const counter = window as unknown as { __HISTORY_FETCH_COUNT__?: number };
    counter.__HISTORY_FETCH_COUNT__ =
      (counter.__HISTORY_FETCH_COUNT__ ?? 0) + 1;
  }
  if (delayMs > 0 && isVisibleOlderHistoryPage) {
    const probe = window as unknown as {
      __HISTORY_INFLIGHT__?: number;
      __HISTORY_INFLIGHT_PEAK__?: number;
    };
    probe.__HISTORY_INFLIGHT__ = (probe.__HISTORY_INFLIGHT__ ?? 0) + 1;
    probe.__HISTORY_INFLIGHT_PEAK__ = Math.max(
      probe.__HISTORY_INFLIGHT_PEAK__ ?? 0,
      probe.__HISTORY_INFLIGHT__,
    );
    window.setTimeout(() => {
      probe.__HISTORY_INFLIGHT__ = (probe.__HISTORY_INFLIGHT__ ?? 1) - 1;
      emit();
    }, delayMs);
    return;
  }

  emit();
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

function emitMockGlobalEvent(event: RelayEvent) {
  for (const socket of mockSockets.values()) {
    for (const [subId, subscription] of socket.subscriptions) {
      if (subscription.kinds && !subscription.kinds.includes(event.kind)) {
        continue;
      }
      sendWsText(socket.handler, ["EVENT", subId, event]);
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

function resetMockUserStatuses() {
  mockUserStatuses.length = 0;
}

function recordMockUserStatus(event: RelayEvent) {
  const dTag = event.tags.find((tag) => tag[0] === "d")?.[1];
  if (dTag) {
    const index = mockUserStatuses.findIndex(
      (stored) =>
        stored.pubkey.toLowerCase() === event.pubkey.toLowerCase() &&
        stored.tags.some((tag) => tag[0] === "d" && tag[1] === dTag),
    );
    if (index >= 0) {
      mockUserStatuses.splice(index, 1);
    }
  }

  mockUserStatuses.push(event);
}

function filterMockUserStatuses(filter: MockFilter) {
  const authors = filter.authors?.map((author) => author.toLowerCase());
  const dTags = filter["#d"];

  return mockUserStatuses
    .filter((event) => {
      if (authors && !authors.includes(event.pubkey.toLowerCase())) {
        return false;
      }
      if (
        dTags &&
        !event.tags.some((tag) => tag[0] === "d" && dTags.includes(tag[1]))
      ) {
        return false;
      }
      return true;
    })
    .sort((a, b) => b.created_at - a.created_at);
}

function emitMockChannelMessage(
  channelId: string,
  content: string,
  parentEventId?: string | null,
  pubkey?: string,
  kind?: number,
  mentionPubkeys?: string[],
  extraTags?: string[][],
  createdAt?: number,
) {
  const eventKind = kind ?? 9;
  if (!parentEventId) {
    const tags = buildTopLevelMessageTags(
      channelId,
      mentionPubkeys,
      pubkey ?? DEFAULT_MOCK_IDENTITY.pubkey,
    );
    if (extraTags) tags.push(...extraTags);
    const event = createMockEvent(eventKind, content, tags, pubkey, createdAt);
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
  const tags = buildReplyMessageTags(
    channelId,
    authorPubkey,
    parentEventId,
    rootEventId,
    mentionPubkeys,
  );
  if (extraTags) tags.push(...extraTags);
  const event = createMockEvent(
    eventKind,
    content,
    tags,
    authorPubkey,
    createdAt,
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

type RawThreadCursor = {
  created_at: number;
  event_id: string;
};

type RawThreadRepliesResponse = {
  events: RelayEvent[];
  next_cursor: RawThreadCursor | null;
};

/**
 * Mirror of the desktop `get_thread_replies` command: return the full reply
 * subtree under a root, chronological (oldest first), excluding the root itself,
 * with gap-free `(created_at, event_id)` keyset paging.
 *
 * The event-id tiebreak is load-bearing — same-second replies must all page
 * through even when they cross a page boundary. This lets a Playwright spec
 * assert the paged union equals the whole subtree, matching the relay contract.
 */
async function handleGetThreadReplies(
  args: {
    rootEventId: string;
    channelId?: string | null;
    limit?: number | null;
    depthLimit?: number | null;
    cursor?: RawThreadCursor | null;
  },
  config: E2eConfig | undefined,
): Promise<RawThreadRepliesResponse> {
  const cap = Math.min(args.limit ?? 200, 500);
  const filter: MockFilter & Record<string, unknown> = {
    "#e": [args.rootEventId],
    depth_limit: args.depthLimit ?? 64,
    kinds: [...TIMELINE_KINDS],
    limit: cap,
  };
  if (args.channelId) {
    filter["#h"] = [args.channelId];
  }
  if (args.cursor) {
    filter.thread_cursor = args.cursor.created_at;
    filter.thread_cursor_id = args.cursor.event_id;
  }
  const identity = getIdentity(config);
  if (
    !isPGatedFilterAuthorized(
      filter,
      identity?.pubkey ?? getMockMemberPubkey(config),
    )
  ) {
    throw new Error(P_GATED_REJECTION_MESSAGE);
  }

  let subtree: RelayEvent[];
  if (!identity) {
    // Mock store: walk the reply forest transitively from the root so nested
    // replies (reply-to-a-reply) are included, matching thread_metadata depth.
    const events = args.channelId
      ? getMockMessageStore(args.channelId)
      : Array.from(mockMessages.values()).flat();
    const byId = new Map(events.map((event) => [event.id, event]));
    const root = byId.get(args.rootEventId);
    const collected: RelayEvent[] = [];
    const included = new Set<string>();
    if (!root) {
      subtree = collected;
    } else {
      const frontier = new Set<string>([root.id]);
      for (;;) {
        let added = false;
        for (const event of events) {
          if (included.has(event.id)) {
            continue;
          }
          const ref = getThreadReferenceFromTags(event.tags);
          if (!ref.parentEventId || !frontier.has(ref.parentEventId)) {
            continue;
          }
          included.add(event.id);
          collected.push(event);
          frontier.add(event.id);
          added = true;
        }
        if (!added) {
          break;
        }
      }
      subtree = collected;
    }
  } else {
    // Config mode: exercise the real bridge thread path over /query.
    const events = await relayQuery(config, [filter]);
    const nextCursor =
      events.length >= cap
        ? {
            created_at: events[events.length - 1].created_at,
            event_id: events[events.length - 1].id,
          }
        : null;
    return { events, next_cursor: nextCursor };
  }

  // Mock mode paging: sort by the composite key, then slice strictly after the
  // cursor so same-second ties can never be skipped across a page boundary.
  subtree.sort(
    (left, right) =>
      left.created_at - right.created_at || left.id.localeCompare(right.id),
  );
  let start = 0;
  if (args.cursor) {
    const cursor = args.cursor;
    start = subtree.findIndex(
      (event) =>
        event.created_at > cursor.created_at ||
        (event.created_at === cursor.created_at &&
          event.id.localeCompare(cursor.event_id) > 0),
    );
    if (start < 0) {
      start = subtree.length;
    }
  }
  const page = subtree.slice(start, start + cap);
  const nextCursor =
    page.length >= cap && start + cap < subtree.length
      ? {
          created_at: page[page.length - 1].created_at,
          event_id: page[page.length - 1].id,
        }
      : null;

  return { events: page, next_cursor: nextCursor };
}

const TIMELINE_KINDS = new Set([
  9,
  40002,
  40008,
  40099,
  43001,
  43002,
  43003,
  43004,
  43005,
  43006,
  KIND_HUDDLE_STARTED,
]);

const KIND_GIFT_WRAP = 1059;
const P_GATED_KINDS = new Set([
  KIND_AGENT_OBSERVER_FRAME,
  KIND_MEMBER_ADDED_NOTIFICATION,
  KIND_MEMBER_REMOVED_NOTIFICATION,
  KIND_GIFT_WRAP,
  KIND_DM_VISIBILITY,
]);
const P_GATED_REJECTION_MESSAGE =
  "restricted: p-gated kinds require #p tag matching your pubkey";

function filterKinds(filter: { kinds?: unknown }): number[] | undefined {
  return Array.isArray(filter.kinds)
    ? filter.kinds.filter((kind): kind is number => typeof kind === "number")
    : undefined;
}

function filterCanMatchPGated(filter: { kinds?: unknown }) {
  const kinds = filterKinds(filter);
  return !kinds || kinds.some((kind) => P_GATED_KINDS.has(kind));
}

function filterHasOwnPTag(filter: { "#p"?: unknown }, pubkey: string) {
  const values = filter["#p"];
  return (
    Array.isArray(values) &&
    values.length > 0 &&
    values.every((value) => value === pubkey)
  );
}

function isPGatedFilterAuthorized(
  filter: { "#p"?: unknown; ids?: unknown; kinds?: unknown },
  pubkey: string,
) {
  if (!filterCanMatchPGated(filter)) {
    return true;
  }

  const kinds = filterKinds(filter);
  const ids = filter.ids;
  const explicitlyDmVisibility = kinds?.includes(KIND_DM_VISIBILITY);
  if (!explicitlyDmVisibility && Array.isArray(ids) && ids.length > 0) {
    return true;
  }

  return filterHasOwnPTag(filter, pubkey);
}

type RawChannelMessagesPageResponse = {
  events: RelayEvent[];
  next_cursor: RawThreadCursor | null;
};

/**
 * Mirror of the desktop `get_channel_messages_before` command: return one
 * keyset page of *top-level* channel history strictly older than a composite
 * `(before, before_id)` cursor, newest first (relay order `created_at DESC,
 * id ASC`).
 *
 * This is the dense-second escape hatch — the id tiebreak is load-bearing so a
 * single `created_at` second denser than one WS page can still be paged
 * through: within a tied second the relay advances via `id > before_id`. Lets a
 * Playwright spec assert the keyset union reaches every top-level message even
 * when a second holds more than one page.
 */
async function handleGetChannelMessagesBefore(
  args: {
    channelId: string;
    before: number;
    beforeId?: string | null;
    limit?: number | null;
  },
  config: E2eConfig | undefined,
): Promise<RawChannelMessagesPageResponse> {
  const cap = Math.min(args.limit ?? 200, 500);
  const identity = getIdentity(config);

  let events: RelayEvent[];
  if (!identity) {
    // Mock store: top-level timeline events for this channel.
    events = getMockMessageStore(args.channelId).filter((event) => {
      if (!TIMELINE_KINDS.has(event.kind)) {
        return false;
      }
      return getThreadReferenceFromTags(event.tags).rootEventId === null;
    });
  } else {
    // Config mode: exercise the real bridge keyset over /query.
    const filter: Record<string, unknown> = {
      "#h": [args.channelId],
      kinds: [...TIMELINE_KINDS],
      until: args.before,
      limit: cap,
    };
    if (args.beforeId) {
      filter.before_id = args.beforeId;
    }
    const page = await relayQuery(config, [filter]);
    const nextCursor =
      page.length >= cap
        ? {
            created_at: page[page.length - 1].created_at,
            event_id: page[page.length - 1].id,
          }
        : null;
    return { events: page, next_cursor: nextCursor };
  }

  // Mock mode paging: relay order (created_at DESC, id ASC), then take the
  // slice strictly older than the composite cursor. Strictly-older means
  // `created_at < before OR (created_at === before AND id > before_id)` — the
  // id tiebreak walks *forward* through a tied second under ASC id order.
  events.sort(
    (left, right) =>
      right.created_at - left.created_at || left.id.localeCompare(right.id),
  );
  const before = args.before;
  const beforeId = args.beforeId ?? null;
  const older = events.filter((event) => {
    if (event.created_at < before) {
      return true;
    }
    if (event.created_at === before && beforeId !== null) {
      return event.id.localeCompare(beforeId) > 0;
    }
    return false;
  });
  const page = older.slice(0, cap);
  const nextCursor =
    page.length >= cap
      ? {
          created_at: page[page.length - 1].created_at,
          event_id: page[page.length - 1].id,
        }
      : null;

  return { events: page, next_cursor: nextCursor };
}

function getMockUserNotes(pubkey: string): RawUserNote[] {
  const now = Math.floor(Date.now() / 1000);

  if (pubkey === DEFAULT_MOCK_IDENTITY.pubkey) {
    // Two named notes plus generated filler so the Pulse feed overflows the
    // viewport — required to exercise windowed scroll + sticky-composer offset.
    const named: RawUserNote[] = [
      {
        id: "mock-note-launch",
        pubkey,
        created_at: now - 20 * 60,
        content: "Shipped the new desktop sidebar polish today.",
        tags: [],
      },
      {
        id: "mock-note-forum",
        pubkey,
        created_at: now - 3 * 60 * 60,
        content: "Forum threads feel like the right home for slower decisions.",
        tags: [],
      },
    ];
    const filler: RawUserNote[] = Array.from({ length: 28 }, (_, index) => ({
      id: `mock-note-filler-${index}`,
      pubkey,
      created_at: now - (4 + index) * 60 * 60,
      content: `Pulse update #${index + 1}: tracking virtualization rollout across desktop surfaces.`,
      tags: [],
    }));
    return [...named, ...filler];
  }

  if (pubkey === ALICE_PUBKEY) {
    return [
      {
        id: "mock-alice-note-release",
        pubkey,
        created_at: now - 45 * 60,
        content: "Release checklist is ready for async feedback.",
        tags: [],
      },
      {
        id: "mock-alice-note-design",
        pubkey,
        created_at: now - 5 * 60 * 60,
        content: "Trying a lighter forum layout for longer-form notes.",
        tags: [],
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
    tags: ev.tags,
  }));
  return { notes, next_cursor: null };
}

async function handleGetGlobalNotes(
  args: { limit?: number | null; before?: number | null } | null,
  config: E2eConfig | undefined,
): Promise<RawUserNotesResponse> {
  const notes = [
    ...getMockUserNotes(DEFAULT_MOCK_IDENTITY.pubkey),
    ...getMockUserNotes(ALICE_PUBKEY),
  ]
    .filter((note) => (args?.before ? note.created_at < args.before : true))
    .sort((left, right) => right.created_at - left.created_at)
    .slice(0, args?.limit ?? 50);

  if (!getIdentity(config)) {
    return { notes, next_cursor: null };
  }

  const events = await relayQuery(config, [
    { kinds: [1], limit: args?.limit ?? 50, until: args?.before ?? undefined },
  ]);
  return {
    notes: events.map((ev) => ({
      id: ev.id,
      pubkey: ev.pubkey,
      content: ev.content,
      created_at: ev.created_at,
      tags: ev.tags,
    })),
    next_cursor: null,
  };
}

function handleGetNotesTimeline(args: {
  pubkeys?: string[];
  limitPerUser?: number | null;
}) {
  const pubkeys = args.pubkeys ?? [];
  const limitPerUser = args.limitPerUser ?? 10;
  const notes = pubkeys
    .flatMap((pubkey) => getMockUserNotes(pubkey).slice(0, limitPerUser))
    .sort((left, right) => right.created_at - left.created_at);
  return { notes, next_cursor: null };
}

function handleGetNote(args: { noteId?: string }) {
  const noteId = args.noteId;
  return (
    [
      ...getMockUserNotes(DEFAULT_MOCK_IDENTITY.pubkey),
      ...getMockUserNotes(ALICE_PUBKEY),
    ].find((note) => note.id === noteId) ?? null
  );
}

function handleGetNoteReactions() {
  return [];
}

function handleGetLikedNotes(): RawUserNotesResponse {
  return { notes: [], next_cursor: null };
}

// A random 64-hex event id, matching the shape of real Nostr event ids
// (sha256 → 64 hex). Most mock events use the 32-hex `createMockEvent` default,
// but kind:7 reactions need a real 64-hex id: the timeline's deletion path only
// accepts 64-hex `e` tags (getDeletionTargets in formatTimelineMessages.ts), so
// a kind:5 targeting a 32-hex reaction id would be silently ignored and the
// reaction pill would never clear on toggle-off.
// --- Mock projects (NIP-34 repo announcements + git activity) ---
// Deterministic fixtures so the Projects view (cards, stat pills, and the
// contribution heatmap) renders with data in screenshots and e2e specs.

const MOCK_PROJECT_SEEDS = [
  {
    dtag: "buzz",
    name: "buzz",
    description:
      "Relay, desktop, and mobile clients for the Buzz workspace platform.",
    owner: MOCK_IDENTITY_PUBKEY,
    contributors: [ALICE_PUBKEY, BOB_PUBKEY, CHARLIE_PUBKEY],
    activityLevel: 4,
  },
  {
    dtag: "relay-tools",
    name: "relay-tools",
    description: "Operator tooling and admin CLI for relay deployments.",
    owner: ALICE_PUBKEY,
    contributors: [MOCK_IDENTITY_PUBKEY, BOB_PUBKEY],
    activityLevel: 2,
  },
  {
    dtag: "design-system",
    name: "design-system",
    description: "Shared UI tokens, typography ramps, and component library.",
    owner: BOB_PUBKEY,
    contributors: [ALICE_PUBKEY],
    activityLevel: 1,
  },
] as const;

const MOCK_PROJECT_SUBJECTS = [
  "Fix reconnect backoff jitter",
  "Polish overview cards",
  "Add contribution heatmap",
  "Refactor filter matching",
  "Speed up event dedup",
  "Handle empty clone URLs",
  "Update onboarding copy",
  "Tighten p-gate checks",
];

const MOCK_PROJECT_KINDS = new Set<number>([
  KIND_REPO_ANNOUNCEMENT,
  KIND_GIT_PATCH,
  KIND_GIT_PULL_REQUEST,
  KIND_GIT_PR_UPDATE,
  KIND_GIT_ISSUE,
  KIND_GIT_STATUS_OPEN,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
]);

function mulberry32(seed: number) {
  let state = seed;
  return () => {
    state |= 0;
    state = (state + 0x6d2b79f5) | 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

let mockProjectEventStore: RelayEvent[] | null = null;

function buildMockProjectEvents(): RelayEvent[] {
  const events: RelayEvent[] = [];
  const daySeconds = 86_400;
  const now = Math.floor(Date.now() / 1000);
  const historyDays = 26 * 7;

  for (const [projectIndex, seed] of MOCK_PROJECT_SEEDS.entries()) {
    const repoAddress = `${KIND_REPO_ANNOUNCEMENT}:${seed.owner}:${seed.dtag}`;
    const authors = [seed.owner, ...seed.contributors];
    const random = mulberry32(projectIndex + 1);

    events.push(
      createMockEvent(
        KIND_REPO_ANNOUNCEMENT,
        seed.description,
        [
          ["d", seed.dtag],
          ["name", seed.name],
          ["description", seed.description],
          ["clone", `https://relay.example.com/git/${seed.dtag}.git`],
          ...seed.contributors.map((pubkey) => ["p", pubkey]),
        ],
        seed.owner,
        now - (historyDays + 30 + projectIndex) * daySeconds,
        `mock-project-${seed.dtag}`.replace(/[^a-zA-Z0-9]/g, ""),
      ),
    );

    for (let dayOffset = historyDays; dayOffset >= 0; dayOffset -= 1) {
      // Roughly half the days are quiet; busy days scale with activityLevel.
      if (random() < 0.5) continue;
      const dayEventCount = 1 + Math.floor(random() * seed.activityLevel);

      for (let index = 0; index < dayEventCount; index += 1) {
        const createdAt =
          now - dayOffset * daySeconds - Math.floor(random() * 10) * 3_600;
        const author = authors[Math.floor(random() * authors.length)];
        const subject =
          MOCK_PROJECT_SUBJECTS[
            Math.floor(random() * MOCK_PROJECT_SUBJECTS.length)
          ];
        const commitHash = `${seed.dtag}${dayOffset}x${index}`
          .padEnd(40, "0")
          .slice(0, 40);
        const roll = random();
        const kind =
          roll < 0.7
            ? KIND_GIT_PATCH
            : roll < 0.85
              ? KIND_GIT_PULL_REQUEST
              : KIND_GIT_ISSUE;
        const tags = [
          ["a", repoAddress],
          ["subject", subject],
          ...(kind === KIND_GIT_ISSUE ? [] : [["c", commitHash]]),
        ];

        events.push(createMockEvent(kind, subject, tags, author, createdAt));
      }
    }
  }

  return events;
}

function getMockProjectEventStore(): RelayEvent[] {
  mockProjectEventStore ??= buildMockProjectEvents();
  return mockProjectEventStore;
}

function filterMockProjectEvents(filter: MockFilter): RelayEvent[] {
  const authors = filter.authors?.map((author) => author.toLowerCase());
  return getMockProjectEventStore()
    .filter((event) => {
      if (filter.kinds && !filter.kinds.includes(event.kind)) return false;
      if (authors && !authors.includes(event.pubkey.toLowerCase())) {
        return false;
      }
      if (
        filter["#d"] &&
        !event.tags.some(
          (tag) => tag[0] === "d" && filter["#d"]?.includes(tag[1]),
        )
      ) {
        return false;
      }
      if (
        filter["#a"] &&
        !event.tags.some(
          (tag) => tag[0] === "a" && filter["#a"]?.includes(tag[1]),
        )
      ) {
        return false;
      }
      return true;
    })
    .sort((left, right) => right.created_at - left.created_at)
    .slice(0, filter.limit ?? 500);
}

function mockEventId(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function createMockEvent(
  kind: number,
  content: string,
  tags: string[][],
  pubkey = DEFAULT_MOCK_IDENTITY.pubkey,
  createdAt = Math.floor(Date.now() / 1000),
  id = crypto.randomUUID().replace(/-/g, ""),
): RelayEvent {
  return {
    id,
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
  if (
    !filters.every((filter) =>
      isPGatedFilterAuthorized(filter, identity.pubkey),
    )
  ) {
    throw new Error(P_GATED_REJECTION_MESSAGE);
  }

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
  const channelsReadError = config?.mock?.channelsReadError;
  if (channelsReadError) {
    throw new Error(channelsReadError);
  }

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

  // NIP-DV: query the viewer's latest DM visibility snapshot (kind:30622).
  // The snapshot is `#p`-gated to its owner, so we query by `#p`=my pubkey.
  // Its `h` tags are the DM channel ids to hide from the sidebar.
  const visibilityEvents = await relayQuery(config, [
    { kinds: [KIND_DM_VISIBILITY], "#p": [identity.pubkey], limit: 1 },
  ]);
  const latestVisibility = visibilityEvents.reduce<RelayEvent | null>(
    (latest, ev) =>
      !latest || ev.created_at > latest.created_at ? ev : latest,
    null,
  );
  const hiddenDms = new Set(
    ((latestVisibility?.tags ?? []) as string[][])
      .filter((t) => t[0] === "h")
      .map((t) => t[1]),
  );

  // Convert kind:39000 events to the RawChannel shape the frontend expects.
  return metaEvents
    .map((ev) => {
      const tags = (ev.tags ?? []) as string[][];
      const getTag = (name: string) =>
        tags.find((t) => t[0] === name)?.[1] ?? null;
      const channelId = getTag("d") ?? "";
      const channelType = getTag("t") ?? "stream";
      const isPrivate = tags.some((t) => t[0] === "private");
      const isArchived = tags.some(
        (t) => t[0] === "archived" && t[1] === "true",
      );

      // Get participant pubkeys from the membership event for this channel
      const memberEvent = memberEvents.find((me) =>
        (me.tags ?? []).some(
          (t: string[]) => t[0] === "d" && t[1] === channelId,
        ),
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
    })
    .filter((c) => c.channel_type !== "dm" || !hiddenDms.has(c.id));
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
      nip05_handle: null,
      owner_pubkey: null,
    };
  }
  const content = JSON.parse(events[0].content ?? "{}");
  return {
    pubkey: identity.pubkey,
    display_name: content.display_name ?? content.name ?? null,
    about: content.about ?? null,
    avatar_url: content.picture ?? null,
    nip05_handle: content.nip05 ?? null,
    owner_pubkey: null,
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
    const profileUpdateErrors = config?.mock?.profileUpdateErrors;
    const nextProfileUpdateError = profileUpdateErrors?.shift();
    if (nextProfileUpdateError) {
      throw new Error(nextProfileUpdateError);
    }

    if (profileUpdateError) {
      if (config?.mock) {
        config.mock.profileUpdateError = undefined;
      }
      throw new Error(profileUpdateError);
    }

    const profile = ensureMockProfile(config);
    const hasDisplayNameUpdate = typeof args.displayName === "string";
    const hasAvatarUrlUpdate = typeof args.avatarUrl === "string";
    const hasAboutUpdate = typeof args.about === "string";
    const hasNip05HandleUpdate = typeof args.nip05Handle === "string";
    const nextDisplayName = args.displayName?.trim() ?? "";
    const nextAvatarUrl = args.avatarUrl?.trim() ?? "";
    const nextAbout = args.about?.trim() ?? "";
    const nextNip05Handle = args.nip05Handle?.trim() ?? "";

    if (hasDisplayNameUpdate && nextDisplayName !== profile.display_name) {
      profile.display_name = nextDisplayName || null;
      applyMockDisplayName(profile.pubkey, profile.display_name);
    }
    if (hasAvatarUrlUpdate && nextAvatarUrl !== profile.avatar_url) {
      profile.avatar_url = nextAvatarUrl || null;
    }
    if (hasAboutUpdate && nextAbout !== profile.about) {
      profile.about = nextAbout || null;
    }
    if (hasNip05HandleUpdate && nextNip05Handle !== profile.nip05_handle) {
      profile.nip05_handle = nextNip05Handle || null;
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
    nip05_handle: updated.nip05 ?? null,
    owner_pubkey: null,
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
      nip05_handle: null,
      owner_pubkey: null,
    };
  }
  const content = JSON.parse(events[0].content ?? "{}");
  return {
    pubkey: targetPubkey,
    display_name: content.display_name ?? content.name ?? null,
    about: content.about ?? null,
    avatar_url: content.picture ?? null,
    nip05_handle: content.nip05 ?? null,
    owner_pubkey: null,
  };
}

async function handleGetUsersBatch(
  args: {
    pubkeys: string[];
  },
  config: E2eConfig | undefined,
) {
  const usersBatchDelayMs = config?.mock?.usersBatchDelayMs ?? 0;
  if (usersBatchDelayMs > 0) {
    await new Promise<void>((resolve) => {
      window.setTimeout(resolve, usersBatchDelayMs);
    });
  }

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
        owner_pubkey: profile.owner_pubkey,
        is_agent: profile.is_agent ?? false,
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
      owner_pubkey:
        ((ev.tags ?? []) as string[][]).find(
          (tag) => Array.isArray(tag) && tag[0] === "auth" && tag.length === 4,
        )?.[1] ?? null,
      is_agent: Array.isArray(ev.tags)
        ? ev.tags.some(
            (tag) =>
              Array.isArray(tag) && tag[0] === "auth" && tag.length === 4,
          )
        : false,
    };
  }
  for (const pubkey of args.pubkeys) {
    const normalizedPubkey = pubkey.toLowerCase();
    if (found.has(normalizedPubkey)) {
      continue;
    }

    const profile = getMockProfileByPubkey(normalizedPubkey);
    if (!profile) {
      continue;
    }

    found.add(normalizedPubkey);
    profiles[normalizedPubkey] = {
      display_name: profile.display_name,
      avatar_url: profile.avatar_url,
      nip05_handle: profile.nip05_handle,
      owner_pubkey: profile.owner_pubkey,
      is_agent: profile.is_agent ?? false,
    };
  }
  const missing = args.pubkeys.filter((p) => !found.has(p.toLowerCase()));
  return { profiles, missing };
}

async function handleSearchUsers(
  args: {
    query: string;
    limit?: number;
    cursor?: string | null;
  },
  config: E2eConfig | undefined,
) {
  const userSearchDelayMs = config?.mock?.userSearchDelayMs ?? 0;
  if (userSearchDelayMs > 0) {
    await new Promise<void>((resolve) => {
      window.setTimeout(resolve, userSearchDelayMs);
    });
  }

  const identity = getIdentity(config);
  if (!identity) {
    const normalizedQuery = args.query.trim().toLowerCase();

    const limit = args.limit ?? 8;
    const page = Math.max(Number(args.cursor ?? 1) || 1, 1);
    const allResults = listMockProfiles()
      .filter((profile) => {
        if (normalizedQuery.length === 0) {
          return true;
        }

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
      });
    const results = allResults
      .slice((page - 1) * limit, page * limit)
      .map((profile) => ({
        pubkey: profile.pubkey,
        display_name: profile.display_name,
        avatar_url: profile.avatar_url,
        nip05_handle: profile.nip05_handle,
        owner_pubkey: profile.owner_pubkey,
        is_agent: profile.is_agent ?? false,
      }));

    return {
      users: results,
      next_cursor: page * limit < allResults.length ? String(page + 1) : null,
    } satisfies RawSearchUsersResponse;
  }

  // NIP-50 search on kind:0 profiles
  const limit = args.limit ?? 8;
  const normalizedQuery = args.query.trim();
  const page = Math.max(Number(args.cursor ?? 1) || 1, 1);
  const filter =
    normalizedQuery.length === 0
      ? { kinds: [0], limit, page }
      : { kinds: [0], search: args.query, limit, page };
  const events = await relayQuery(config, [filter]);
  const users = events.map((ev) => {
    const content = JSON.parse(ev.content ?? "{}");
    return {
      pubkey: ev.pubkey ?? "",
      display_name: content.display_name ?? content.name ?? null,
      avatar_url: content.picture ?? null,
      nip05_handle: content.nip05 ?? null,
      owner_pubkey:
        ((ev.tags ?? []) as string[][]).find(
          (tag) => Array.isArray(tag) && tag[0] === "auth" && tag.length === 4,
        )?.[1] ?? null,
      is_agent: Array.isArray(ev.tags)
        ? ev.tags.some(
            (tag) =>
              Array.isArray(tag) && tag[0] === "auth" && tag.length === 4,
          )
        : false,
    };
  });
  return {
    users,
    next_cursor: users.length >= limit ? String(page + 1) : null,
  };
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

  // Presence is ephemeral (kind:20001) — mock returns from in-memory map.
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
  const delayMs = config?.mock?.openDmDelayMs ?? 0;
  if (delayMs > 0) {
    await new Promise((resolve) => window.setTimeout(resolve, delayMs));
  }

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
    is_agent: (t[3] ?? t[2]) === "bot",
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
    visibility?: "open" | "private";
    ttlSeconds?: number | null;
  },
  config: E2eConfig | undefined,
) {
  const delayMs = config?.mock?.updateChannelDelayMs ?? 0;
  if (delayMs > 0) {
    await new Promise((resolve) => window.setTimeout(resolve, delayMs));
  }

  const identity = getIdentity(config);
  if (!identity) {
    const channel = getMockChannel(args.channelId);
    if (args.name !== undefined) {
      channel.name = args.name;
    }
    if (args.description !== undefined) {
      channel.description = args.description;
    }
    if (args.visibility !== undefined) {
      channel.visibility = args.visibility;
    }
    if (args.ttlSeconds !== undefined) {
      channel.ttl_seconds = args.ttlSeconds;
      channel.ttl_deadline =
        args.ttlSeconds === null
          ? null
          : new Date(Date.now() + args.ttlSeconds * 1000).toISOString();
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
  if (args.visibility !== undefined) {
    tags.push(["visibility", args.visibility]);
  }
  if (args.ttlSeconds !== undefined) {
    tags.push(["ttl", args.ttlSeconds === null ? "" : String(args.ttlSeconds)]);
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
  const ttlTag = getTag("ttl");
  const ttlSeconds = ttlTag === null || ttlTag === "" ? null : Number(ttlTag);
  return {
    id: args.channelId,
    name: getTag("name") ?? "",
    description: getTag("about") ?? null,
    channel_type: getTag("t") ?? "stream",
    visibility: getTag("visibility") ?? "open",
    topic: getTag("topic") ?? null,
    purpose: getTag("purpose") ?? null,
    member_count: 0,
    role: "owner",
    archived_at: null,
    ttl_seconds: ttlSeconds,
    ttl_deadline:
      ttlSeconds === null
        ? null
        : new Date(Date.now() + ttlSeconds * 1000).toISOString(),
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

type MockUpdaterChannel = {
  onmessage?: (event: { event: "Finished" }) => void;
};

function notifyUpdaterFinished(payload: unknown) {
  const channel = (payload as { onEvent?: MockUpdaterChannel } | null)?.onEvent;
  channel?.onmessage?.({ event: "Finished" });
}

function handleUpdaterCheck(config: E2eConfig | undefined) {
  if (!config?.mock?.updateAvailable) {
    return null;
  }

  const version = config.mock.updateVersion ?? "0.3.18";

  return {
    rid: 42,
    currentVersion: "0.3.17",
    version,
    date: "2026-06-12T00:00:00Z",
    body: `Mock update ${version}`,
    rawJson: null,
  };
}

async function handleUpdaterDownloadAndInstall(
  payload: unknown,
  config: E2eConfig | undefined,
) {
  const delayMs = config?.mock?.updateDownloadDelayMs ?? 0;

  if (delayMs > 0) {
    await new Promise((resolve) => window.setTimeout(resolve, delayMs));
  }

  notifyUpdaterFinished(payload);
  return null;
}

async function handleRestart(config: E2eConfig | undefined) {
  const delayMs = config?.mock?.restartDelayMs ?? 0;

  if (delayMs > 0) {
    await new Promise((resolve) => window.setTimeout(resolve, delayMs));
  }

  return null;
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
  const addChannelMembersDelayMs = config?.mock?.addChannelMembersDelayMs ?? 0;
  if (addChannelMembersDelayMs > 0) {
    await new Promise((resolve) =>
      window.setTimeout(resolve, addChannelMembersDelayMs),
    );
  }
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
        is_agent:
          args.role === "bot" ||
          mockAgentPubkeys.has(pubkey) ||
          mockManagedAgents.some((agent) => agent.pubkey === pubkey),
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
  const feedReadError = config?.mock?.feedReadError;
  if (feedReadError) {
    throw new Error(feedReadError);
  }

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

async function delayAgentList(config: E2eConfig | undefined) {
  const agentListDelayMs = config?.mock?.agentListDelayMs ?? 0;
  if (agentListDelayMs > 0) {
    await new Promise<void>((resolve) => {
      window.setTimeout(resolve, agentListDelayMs);
    });
  }
}

async function handleListRelayAgents(
  config: E2eConfig | undefined,
): Promise<RawRelayAgent[]> {
  await delayAgentList(config);
  syncMockRelayAgentsFromManagedAgents();
  return mockRelayAgents.map(cloneRelayAgent);
}

async function handleDiscoverAcpRuntimes(
  config: E2eConfig | undefined,
): Promise<RawAcpRuntimeCatalogEntry[]> {
  const configured = config?.mock?.acpRuntimesCatalog;
  if (configured) {
    return configured;
  }
  return [
    {
      id: "goose",
      label: "Goose",
      avatar_url: "",
      availability: "available",
      command: "goose",
      binary_path: "/usr/local/bin/goose",
      default_args: ["acp"],
      mcp_command: null,
      install_hint: "Install Goose via the official install script.",
      install_instructions_url: "https://block.github.io/goose/",
      can_auto_install: true,
      underlying_cli_path: null,
    },
    {
      id: "claude",
      label: "Claude Code",
      avatar_url: "",
      availability: "adapter_missing",
      command: null,
      binary_path: null,
      default_args: [],
      mcp_command: null,
      install_hint: "Install the Claude Code ACP adapter via npm.",
      install_instructions_url:
        "https://www.npmjs.com/package/@anthropic-ai/claude-agent-acp",
      can_auto_install: true,
      underlying_cli_path: "/usr/local/bin/claude",
    },
    {
      id: "codex",
      label: "Codex",
      avatar_url: "",
      availability: "not_installed",
      command: null,
      binary_path: null,
      default_args: [],
      mcp_command: null,
      install_hint:
        "The codex-acp adapter must be built from source. See the GitHub repo.",
      install_instructions_url: "https://github.com/openai/codex",
      can_auto_install: false,
      underlying_cli_path: null,
    },
    {
      id: "buzz-agent",
      label: "Buzz Agent",
      avatar_url: "",
      availability: "available",
      command: "buzz-agent",
      binary_path: "/usr/local/bin/buzz-agent",
      default_args: [],
      mcp_command: "buzz-dev-mcp",
      install_hint: "Ships with the Buzz desktop app.",
      install_instructions_url: "https://github.com/block/buzz",
      can_auto_install: false,
      underlying_cli_path: null,
    },
  ];
}

async function handleInstallAcpRuntime(
  args: {
    runtimeId?: string;
  },
  config: E2eConfig | undefined,
): Promise<RawInstallRuntimeResult> {
  const configured = config?.mock?.installAcpRuntimeResult;
  if (configured) {
    return configured;
  }
  return {
    success: true,
    steps: [
      {
        step: "adapter",
        command: `mock install ${args.runtimeId ?? "unknown"}`,
        success: true,
        stdout: "mock: installed successfully",
        stderr: "",
        exit_code: 0,
      },
    ],
  };
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
        configuredPrereqs?.acp?.command ?? args.input?.acpCommand ?? "buzz-acp",
      resolved_path:
        configuredPrereqs?.acp?.resolvedPath ??
        "/Users/wesb/dev/buzz/target/debug/buzz-acp",
      available: configuredPrereqs?.acp?.available ?? true,
    },
    mcp: {
      command: configuredPrereqs?.mcp?.command ?? args.input?.mcpCommand ?? "",
      resolved_path: configuredPrereqs?.mcp?.resolvedPath ?? "",
      available: configuredPrereqs?.mcp?.available ?? true,
    },
  };
}

async function handleListManagedAgents(
  config: E2eConfig | undefined,
): Promise<RawManagedAgent[]> {
  await delayAgentList(config);
  return mockManagedAgents.map(cloneManagedAgent);
}

function isAgentMemoryListing(
  value: RawAgentMemoryListing | Record<string, RawAgentMemoryListing>,
): value is RawAgentMemoryListing {
  return (
    "memories" in value &&
    Array.isArray(value.memories) &&
    "truncated" in value &&
    "fetchedAt" in value
  );
}

async function handleGetAgentMemory(
  args: { agentPubkey?: string },
  config: E2eConfig | undefined,
): Promise<RawAgentMemoryListing> {
  const pubkey = args.agentPubkey?.toLowerCase();
  if (!pubkey) {
    throw new Error("mock get_agent_memory: missing agent pubkey");
  }

  const isManagedAgent = mockManagedAgents.some(
    (agent) => agent.pubkey.toLowerCase() === pubkey,
  );
  if (!isManagedAgent) {
    throw new Error(`mock get_agent_memory: unmanaged agent ${pubkey}`);
  }

  const configuredMemory = config?.mock?.agentMemory;
  if (!configuredMemory) {
    return {
      core: null,
      memories: [],
      truncated: false,
      fetchedAt: Math.floor(Date.now() / 1000),
    };
  }

  if (isAgentMemoryListing(configuredMemory)) {
    return cloneAgentMemoryListing(configuredMemory);
  }

  const listing =
    configuredMemory[pubkey] ?? configuredMemory[args.agentPubkey ?? ""];
  return listing
    ? cloneAgentMemoryListing(listing)
    : {
        core: null,
        memories: [],
        truncated: false,
        fetchedAt: Math.floor(Date.now() / 1000),
      };
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
    source_team: null,
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
      `${persona.display_name} is not in My Agents. Choose it from Agent Catalog first.`,
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
    source_dir: null,
    is_symlink: false,
    symlink_target: null,
    version: null,
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

async function handlePickTeamDirectory(): Promise<string | null> {
  return "/Users/dev/agents/new-team";
}

async function handleInstallTeamFromDirectory(args: {
  path: string;
  symlink: boolean;
}): Promise<RawTeam> {
  const now = new Date().toISOString();
  const team: RawTeam = {
    id: crypto.randomUUID(),
    name: "Installed Team",
    description: null,
    persona_ids: [],
    is_builtin: false,
    source_dir: args.path,
    is_symlink: args.symlink,
    symlink_target: args.symlink ? args.path : null,
    version: null,
    created_at: now,
    updated_at: now,
  };
  mockTeams.push(team);
  return { ...team, persona_ids: [...team.persona_ids] };
}

async function handleSyncTeamDirectory(args: { teamId: string }): Promise<{
  personas_added: string[];
  personas_removed: string[];
  personas_updated: string[];
  metadata_changed: boolean;
}> {
  const team = mockTeams.find((candidate) => candidate.id === args.teamId);
  if (!team) {
    throw new Error(`Team ${args.teamId} not found.`);
  }
  return {
    personas_added: [],
    personas_removed: [],
    personas_updated: [],
    metadata_changed: false,
  };
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
    avatar_ref: string | null;
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
        avatar_ref: null,
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

async function handleCreateManagedAgent(
  args: {
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
  },
  config: E2eConfig | undefined,
): Promise<RawCreateManagedAgentResponse> {
  const delayMs = config?.mock?.createManagedAgentDelayMs ?? 0;
  if (delayMs > 0) {
    await new Promise((resolve) => window.setTimeout(resolve, delayMs));
  }

  if (args.input.personaId) {
    ensureMockPersonaIsActive(args.input.personaId);
  }
  const personaAvatarUrl =
    args.input.personaId === undefined
      ? null
      : (mockPersonas.find((persona) => persona.id === args.input.personaId)
          ?.avatar_url ?? null);
  const avatarUrl = args.input.avatarUrl?.trim() || personaAvatarUrl;
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
    acp_command: args.input.acpCommand ?? "buzz-acp",
    agent_command: args.input.agentCommand ?? "goose",
    agent_args:
      args.input.agentArgs && args.input.agentArgs.length > 0
        ? [...args.input.agentArgs]
        : ["acp"],
    mcp_command: args.input.mcpCommand ?? "",
    turn_timeout_seconds: args.input.turnTimeoutSeconds ?? 320,
    idle_timeout_seconds: args.input.idleTimeoutSeconds ?? null,
    max_turn_duration_seconds: args.input.maxTurnDurationSeconds ?? null,
    parallelism: args.input.parallelism ?? 1,
    system_prompt: args.input.systemPrompt?.trim() || null,
    avatar_url: avatarUrl,
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
      `buzz-acp starting: relay=${args.input.relayUrl ?? DEFAULT_RELAY_WS_URL} agent_pubkey=${pubkey} parallelism=${args.input.parallelism ?? 1}`,
      args.input.systemPrompt?.trim()
        ? `system prompt override configured (${args.input.systemPrompt.trim().length} chars)`
        : "system prompt override not set",
      args.input.spawnAfterCreate
        ? "connected to relay at ws://localhost:3000"
        : "profile created; harness not started",
    ],
  };

  mockManagedAgents.unshift(managedAgent);
  applyMockDisplayName(pubkey, name);
  mockAgentPubkeys.add(pubkey);
  mockProfiles.set(pubkey, {
    pubkey,
    display_name: name,
    avatar_url: avatarUrl,
    about: args.input.systemPrompt?.trim() || null,
    nip05_handle: null,
    owner_pubkey: MOCK_IDENTITY_PUBKEY,
    is_agent: true,
  });
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

function isRelayMeshManagedAgent(agent: MockManagedAgent): boolean {
  const env = agent.env_vars ?? {};
  return (
    agent.backend.type === "local" &&
    env.BUZZ_AGENT_PROVIDER === "openai" &&
    env.OPENAI_COMPAT_BASE_URL?.replace(/\/+$/, "") ===
      "http://127.0.0.1:9337/v1" &&
    env.OPENAI_COMPAT_API_KEY === "buzz-mesh-local"
  );
}

async function handleStartManagedAgent(args: {
  pubkey: string;
}): Promise<RawManagedAgent> {
  const agent = getMockManagedAgent(args.pubkey);
  if (isRelayMeshManagedAgent(agent)) {
    // Model the backend start preflight (ensure_relay_mesh_for_record): a
    // saved relay-mesh agent re-resolves a live serve target for its model
    // and only fails when no peer currently serves it.
    const modelId = agent.env_vars?.OPENAI_COMPAT_MODEL;
    const hasLiveTarget =
      mockMeshState.admitted &&
      mockMeshState.models.some((model) => model.id === modelId);
    if (!hasLiveTarget) {
      throw new Error(
        "relay mesh agents cannot be started from saved state because no live serve target is available for this model. Start serving on a mesh peer, or create a new agent with Run on relay mesh selected to refresh the target for http://127.0.0.1:9337/v1.",
      );
    }
  }

  const now = new Date().toISOString();
  if (agent.backend.type === "provider") {
    agent.status = "deployed";
    agent.pid = null;
    agent.backend_agent_id =
      agent.backend_agent_id ?? `mock-provider-${agent.pubkey.slice(0, 12)}`;
  } else {
    agent.status = "running";
    agent.pid = agent.pid ?? 42000 + mockManagedAgents.indexOf(agent);
  }
  agent.updated_at = now;
  agent.last_started_at = now;
  agent.last_error = null;
  agent.log_lines.push(
    agent.backend.type === "provider"
      ? `deployed mock provider harness at ${now}`
      : `started mock harness at ${now}`,
  );
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
    for (const [channelId, events] of mockMessages) {
      const channel = mockChannels.find(
        (candidate) => candidate.id === channelId,
      );
      for (const event of events) {
        mockHits.push({
          event_id: event.id,
          content: event.content,
          kind: event.kind,
          pubkey: event.pubkey,
          channel_id: channelId,
          channel_name: channel?.name ?? null,
          created_at: event.created_at,
          score: 1,
        });
      }
    }

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

/**
 * Descriptors returned by the mocked upload commands. A spec can override via
 * `MockBridgeOptions.uploadDescriptors`; otherwise we return a single generic
 * PDF so the file-attachment flow (chip → send → FileCard) can be exercised
 * out of the box.
 */
async function resolveMockUploadDescriptors(
  config: E2eConfig | undefined,
): Promise<RawBlobDescriptor[]> {
  const delayMs = config?.mock?.uploadDelayMs ?? 0;
  if (delayMs > 0) {
    await new Promise((resolve) => window.setTimeout(resolve, delayMs));
  }

  const configured = config?.mock?.uploadDescriptors;
  // `undefined` means "not configured" → default PDF. An explicit `[]` is a
  // valid override (e.g. modelling a picker cancel / no-files-selected), so it
  // must pass through rather than fall back to the default.
  if (configured !== undefined) return configured;
  return [
    {
      url: `https://mock.relay/media/${"a".repeat(64)}.pdf`,
      sha256: "a".repeat(64),
      size: 12345,
      type: "application/pdf",
      uploaded: Math.floor(Date.now() / 1000),
      filename: "quarterly-report.pdf",
    },
  ];
}

async function handleSendChannelMessage(
  args: {
    channelId: string;
    content: string;
    parentEventId?: string | null;
    kind?: number | null;
    mentionPubkeys?: string[];
    mediaTags?: string[][] | null;
    emojiTags?: string[][] | null;
  },
  config: E2eConfig | undefined,
): Promise<RawSendChannelMessageResponse> {
  const kind = args.kind ?? 9;
  const sendMessageDelayMs = config?.mock?.sendMessageDelayMs ?? 0;
  if (sendMessageDelayMs > 0) {
    await new Promise((resolve) =>
      window.setTimeout(resolve, sendMessageDelayMs),
    );
  }

  // NIP-92 imeta attachments. The real relay echoes these back on the stored
  // event; mirror that here so attachment renderers (FileCard, images, video)
  // have the imeta tags they key on. `null`/empty → no extra tags.
  const mediaTags = args.mediaTags ?? [];
  // NIP-30 custom-emoji tags ride their own validated arg server-side; the
  // relay echoes them back on the stored event too, so mirror that here so the
  // emoji renderer keeps resolving `:shortcode:` after the round-trip.
  const emojiTags = args.emojiTags ?? [];
  // Both kinds end up on the stored event's tag set, just like the real relay.
  const extraTags = [...mediaTags, ...emojiTags];
  const identity = getIdentity(config);
  if (!identity) {
    const createdAt = Math.floor(Date.now() / 1000);
    const mockPubkey = getMockMemberPubkey(config);

    if (!args.parentEventId) {
      const event = createMockEvent(kind, args.content, [
        ...buildTopLevelMessageTags(
          args.channelId,
          args.mentionPubkeys,
          mockPubkey,
        ),
        ...extraTags,
      ]);
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
      tags: [
        ...buildReplyMessageTags(
          args.channelId,
          mockPubkey,
          args.parentEventId,
          rootEventId,
          args.mentionPubkeys,
        ),
        ...extraTags,
      ],
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
    tags: [...tags, ...extraTags],
  });

  return {
    event_id: result.event_id,
    parent_event_id: args.parentEventId ?? null,
    root_event_id: args.parentEventId ?? null,
    depth: args.parentEventId ? 1 : 0,
    created_at: Math.floor(Date.now() / 1000),
  };
}

async function handleSendManagedAgentChannelMessage(
  args: {
    agentPubkey: string;
    channelId: string;
    content: string;
    marker?: string | null;
  },
  _config: E2eConfig | undefined,
): Promise<RawSendChannelMessageResponse> {
  const agent = getMockManagedAgent(args.agentPubkey);
  const marker = args.marker?.trim();
  if (marker) {
    const existing = getMockMessageStore(args.channelId).find(
      (event) =>
        event.pubkey === agent.pubkey &&
        event.tags.some((tag) => tag[0] === "client" && tag[1] === marker),
    );
    if (existing) {
      return {
        event_id: existing.id,
        parent_event_id: null,
        root_event_id: null,
        depth: 0,
        created_at: existing.created_at,
      };
    }
  }

  const createdAt = Math.floor(Date.now() / 1000);
  const event = createMockEvent(
    9,
    args.content.trim(),
    [["h", args.channelId], ...(marker ? [["client", marker]] : [])],
    agent.pubkey,
    createdAt,
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

/**
 * Mock the `delete_message` Tauri command. Removes the event from the
 * in-memory mock store so the query-cache invalidation in
 * `useDeleteMessageMutation.onSuccess` (which filters by eventId) finds
 * nothing to keep, and the row disappears from the timeline.
 */
function handleDeleteMessage(args: {
  channelId: string;
  eventId: string;
}): void {
  const history = mockMessages.get(args.channelId);
  if (history) {
    const index = history.findIndex((ev) => ev.id === args.eventId);
    if (index !== -1) history.splice(index, 1);
  }
}

/**
 * Mock the `edit_message` Tauri command. Mirrors the real Rust command
 * (`build_message_edit`): emit a kind:40003 edit event carrying `["e", target]`
 * plus the new content, media (imeta) tags, and NIP-30 emoji tags. The timeline
 * (`formatTimelineMessages`) scans for these edit events and overlays the new
 * content + media/emoji tags onto the original via `applyEditTagOverlay`, so
 * recording + emitting the edit event is all the bridge needs to do — the same
 * path the real relay drives. `null`/empty tag args → no extra tags.
 */
async function handleEditMessage(
  args: {
    channelId: string;
    eventId: string;
    content: string;
    mediaTags?: string[][] | null;
    emojiTags?: string[][] | null;
  },
  config: E2eConfig | undefined,
): Promise<void> {
  const mediaTags = args.mediaTags ?? [];
  const emojiTags = args.emojiTags ?? [];
  const extraTags = [...mediaTags, ...emojiTags];
  const tags = [["h", args.channelId], ["e", args.eventId], ...extraTags];
  const content = args.content.trim();
  const identity = getIdentity(config);

  if (!identity) {
    const editEvent = createMockEvent(
      KIND_STREAM_MESSAGE_EDIT,
      content,
      tags,
      getMockMemberPubkey(config),
    );
    recordMockMessage(args.channelId, editEvent);
    emitMockLiveEvent(args.channelId, editEvent);
    return;
  }

  await submitSignedEvent(config, {
    kind: KIND_STREAM_MESSAGE_EDIT,
    content,
    tags,
  });
}

/** Locate the channel a stored mock event lives in (reactions carry no channel arg). */
function findMockEventChannel(eventId: string): string | undefined {
  for (const [channelId, events] of mockMessages) {
    if (events.some((event) => event.id === eventId)) {
      return channelId;
    }
  }
  return undefined;
}

/**
 * Mock the `add_reaction` Tauri command. Mirrors the real Rust command: a
 * kind:7 whose content is the emoji, plus — for a custom emoji — the NIP-30
 * `["emoji", shortcode, url]` tag (shortcode normalized to match the relay).
 * Recorded into the target's channel store and emitted live so the timeline's
 * reaction aggregation renders the pill (the channel subscription includes
 * kind:7). Unicode reactions carry no emoji tag, like the real command.
 */
async function handleAddReaction(
  args: { eventId: string; emoji: string; emojiUrl?: string | null },
  config: E2eConfig | undefined,
): Promise<void> {
  const channelId = findMockEventChannel(args.eventId);
  if (!channelId) {
    throw new Error(`mock add_reaction: unknown target event ${args.eventId}`);
  }

  const emoji = args.emoji.trim();
  // `h` routes the live event to the channel store (getChannelIdFromTags);
  // `e` names the reaction target. For a custom emoji, the NIP-30
  // `["emoji", shortcode, url]` tag carries the image URL.
  const tags: string[][] = [
    ["h", channelId],
    ["e", args.eventId],
  ];
  if (args.emojiUrl) {
    const shortcode = emoji.replace(/^:+/, "").replace(/:+$/, "").toLowerCase();
    tags.push(["emoji", shortcode, args.emojiUrl]);
  }

  const event = createMockEvent(
    KIND_REACTION,
    emoji,
    tags,
    getMockMemberPubkey(config),
    Math.floor(Date.now() / 1000),
    // 64-hex id so the kind:5 deletion emitted by remove_reaction is accepted
    // by the timeline (getDeletionTargets requires a 64-hex `e` tag).
    mockEventId(),
  );
  recordMockMessage(channelId, event);
  emitMockLiveEvent(channelId, event);
}

/**
 * Mock the `remove_reaction` Tauri command. Finds the active member's own
 * kind:7 for this target+emoji, removes it from the store, and emits a kind:5
 * deletion so the timeline drops the reaction (the real command deletes via a
 * kind:5 too).
 */
async function handleRemoveReaction(
  args: { eventId: string; emoji: string },
  config: E2eConfig | undefined,
): Promise<void> {
  const channelId = findMockEventChannel(args.eventId);
  if (!channelId) {
    return;
  }

  const myPubkey = getMockMemberPubkey(config).toLowerCase();
  const emoji = args.emoji.trim();
  const store = getMockMessageStore(channelId);
  const reaction = store.find(
    (event) =>
      event.kind === KIND_REACTION &&
      event.pubkey.toLowerCase() === myPubkey &&
      event.content.trim() === emoji &&
      event.tags.some((t) => t[0] === "e" && t[1] === args.eventId),
  );
  if (!reaction) {
    return;
  }

  const index = store.indexOf(reaction);
  store.splice(index, 1);

  const deletion = createMockEvent(
    KIND_DELETION,
    "",
    [["e", reaction.id]],
    getMockMemberPubkey(config),
  );
  recordMockMessage(channelId, deletion);
  emitMockLiveEvent(channelId, deletion);
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
        tags: [["h", "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9"]],
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
  if (mockWebsocketSendMutexWedged) {
    return new Promise<number>(() => {});
  }

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
  if (
    getConfig()?.mock?.stallWebsocketSends &&
    args.message?.type !== "Close"
  ) {
    mockWebsocketSendMutexWedged = true;
    return new Promise<void>(() => {});
  }

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
    const filters = rest.slice(1) as MockFilter[];
    if (
      !filters.every((filter) =>
        isPGatedFilterAuthorized(filter, MOCK_IDENTITY_PUBKEY),
      )
    ) {
      sendWsText(socket.handler, ["CLOSED", subId, P_GATED_REJECTION_MESSAGE]);
      return;
    }

    if (subId.startsWith("live-")) {
      // Collect channel IDs from all filters in the REQ
      const channelIds = new Set<string>();
      const kinds = new Set<number>();
      for (const f of filters) {
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

    const filter = rest[1] as MockFilter;
    if (filter.kinds?.includes(13534)) {
      sendWsText(socket.handler, [
        "EVENT",
        subId,
        createMockRelayMembershipEvent(),
      ]);
      sendWsText(socket.handler, ["EOSE", subId]);
      return;
    }

    if (filter.kinds?.includes(KIND_EMOJI_SET)) {
      // Honor `authors` so `fetchOwnEmoji` (authors:[me]) sees only the
      // caller's set, while the union fetch (no authors) sees every member's —
      // matching the real relay and the own-vs-workspace split in the UI.
      const authors = filter.authors?.map((a) => a.toLowerCase());
      for (const emojiEvent of createMockCustomEmojiSetEvents()) {
        if (authors && !authors.includes(emojiEvent.pubkey.toLowerCase())) {
          continue;
        }
        sendWsText(socket.handler, ["EVENT", subId, emojiEvent]);
      }
      sendWsText(socket.handler, ["EOSE", subId]);
      return;
    }

    if (filter.kinds?.includes(KIND_USER_STATUS)) {
      for (const statusEvent of filterMockUserStatuses(filter)) {
        sendWsText(socket.handler, ["EVENT", subId, statusEvent]);
      }
      sendWsText(socket.handler, ["EOSE", subId]);
      return;
    }

    if (filter.kinds?.includes(KIND_EVENT_REMINDER)) {
      const authors = filter.authors?.map((a) => a.toLowerCase());
      for (const event of mockReminderEvents) {
        if (authors && !authors.includes(event.pubkey.toLowerCase())) continue;
        sendWsText(socket.handler, ["EVENT", subId, event]);
      }
      sendWsText(socket.handler, ["EOSE", subId]);
      return;
    }

    if (filter.kinds?.some((kind) => MOCK_PROJECT_KINDS.has(kind))) {
      for (const event of filterMockProjectEvents(filter)) {
        sendWsText(socket.handler, ["EVENT", subId, event]);
      }
      sendWsText(socket.handler, ["EOSE", subId]);
      return;
    }

    const channelId = filter["#h"]?.[0];
    if (!channelId) {
      sendWsText(socket.handler, ["EOSE", subId]);
      return;
    }

    emitMockHistory(socket, subId, channelId, filter);
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

    // Mesh control events (24620 status report, 24621 connect request) are not
    // channel messages — they carry a `p` tag, not an `h` tag. The real relay
    // accepts them after membership/shape checks; the mock just ACKs so legacy
    // direct-TS mesh helpers can proceed. Current create/start flows publish
    // these from the Rust coordinator instead.
    if (event.kind === 24620 || event.kind === 24621) {
      if (
        event.kind === 24621 &&
        !event.tags.some((tag) => tag[0] === "p" && typeof tag[1] === "string")
      ) {
        sendWsText(socket.handler, [
          "OK",
          event.id,
          false,
          "invalid: mesh connect request missing #p target",
        ]);
        return;
      }
      sendWsText(socket.handler, ["OK", event.id, true, ""]);
      return;
    }

    if (event.kind === 30078) {
      sendWsText(socket.handler, ["OK", event.id, true, ""]);
      return;
    }

    if (event.kind === KIND_EVENT_REMINDER) {
      // Upsert by d-tag (replaceable event)
      const dTag = event.tags.find((t) => t[0] === "d")?.[1];
      if (dTag) {
        const idx = mockReminderEvents.findIndex(
          (e) =>
            e.pubkey.toLowerCase() === event.pubkey.toLowerCase() &&
            e.tags.some((t) => t[0] === "d" && t[1] === dTag),
        );
        if (idx >= 0) mockReminderEvents.splice(idx, 1);
      }
      mockReminderEvents.push(event);
      sendWsText(socket.handler, ["OK", event.id, true, ""]);
      return;
    }

    if (event.kind === 20001) {
      const status = event.content;
      if (status === "online" || status === "away" || status === "offline") {
        setMockPresenceStatus(event.pubkey, status);
      }
      emitMockGlobalEvent(event);
      sendWsText(socket.handler, ["OK", event.id, true, ""]);
      return;
    }

    if (event.kind === KIND_USER_STATUS) {
      const hasGeneralDTag = event.tags.some(
        (tag) => tag[0] === "d" && tag[1] === "general",
      );
      if (!hasGeneralDTag) {
        sendWsText(socket.handler, [
          "OK",
          event.id,
          false,
          "invalid: user status missing d tag.",
        ]);
        return;
      }

      recordMockUserStatus(event);
      emitMockGlobalEvent(event);
      sendWsText(socket.handler, ["OK", event.id, true, ""]);
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
  resetMockRelayAgents(config);
  resetMockManagedAgents(config);
  resetMockPersonas(config);
  resetMockTeams();
  seedMockSearchProfiles(config);
  resetMockWorkflows();
  resetMockMesh();
  resetMockUserStatuses();
  mockWebsocketSendMutexWedged = false;
  mockWindows("main");
  window.__BUZZ_E2E_COMMANDS__ = [];
  window.__BUZZ_E2E_COMMAND_PAYLOADS__ = [];
  window.__BUZZ_E2E_COMMAND_LOG__ = [];
  window.__BUZZ_E2E_SIGNED_EVENTS__ = [];
  window.__BUZZ_E2E_WEBVIEW_ZOOM__ = 1;
  window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ = ({
    channelName,
    content,
    parentEventId,
    pubkey,
    kind,
    mentionPubkeys,
    extraTags,
    createdAt,
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
      mentionPubkeys,
      extraTags,
      createdAt,
    );
  };
  window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__ = prependMockHistory;
  window.__BUZZ_E2E_EMIT_MOCK_TYPING__ = ({ channelName, pubkey }) => {
    const channel = mockChannels.find(
      (candidate) => candidate.name === channelName,
    );
    if (!channel) {
      throw new Error(`Mock channel ${channelName} not found.`);
    }

    return emitMockTypingIndicator(channel.id, pubkey ?? CHARLIE_PUBKEY);
  };
  window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__ = ({ channelName, kind }) => {
    const channel = mockChannels.find(
      (candidate) => candidate.name === channelName,
    );
    if (!channel) {
      throw new Error(`Mock channel ${channelName} not found.`);
    }

    return hasMockLiveSubscription(channel.id, kind);
  };
  window.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__ = (item) => {
    const category = item.category === "mention" ? "mentions" : item.category;
    mockFeedOverrides[category].unshift(item);
    window.dispatchEvent(new CustomEvent("buzz:e2e-home-feed-updated"));
    return item;
  };
  window.__BUZZ_E2E_EMIT_MOCK_READ_STATE__ = ({
    clientId,
    contexts,
    createdAt,
    slotId,
  }) => {
    const blob = JSON.stringify({
      v: 1,
      client_id: clientId,
      contexts,
    });
    const event = createMockEvent(
      30078,
      blob,
      [
        ["d", `read-state:${slotId}`],
        ["t", "read-state"],
      ],
      getMockMemberPubkey(config),
      createdAt,
    );
    emitMockLiveEvent(GLOBAL_MOCK_SUBSCRIPTION, event);
    return event;
  };
  window.__BUZZ_E2E_SET_RELAY_CONNECTION_STATE__ = (state) => {
    // Directly emit a connection state change on the relay client singleton,
    // for tests that need to drive degraded relay UI without waiting for the
    // real auth-timeout + reconnect-debounce cycle (~10 s). Reaches the
    // TS-private emitter via a cast so the production class carries no
    // test-only seam.
    (
      relayClient as unknown as {
        connectionStateEmitter: { set: (s: ConnectionState) => void };
      }
    ).connectionStateEmitter.set(state);
  };
  window.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__ = () =>
    relayClient.getConnectionState();

  window.__BUZZ_E2E_SEED_MOCK_REMINDERS__ = (reminders) => {
    mockReminderEvents.length = 0;
    for (const r of reminders) {
      mockReminderEvents.push(r);
    }
  };

  window.__BUZZ_E2E_SET_STALL_WEBSOCKET_SENDS__ = (stall) => {
    const config = getConfig();
    if (!config?.mock) return;
    config.mock.stallWebsocketSends = stall;
    if (!stall) mockWebsocketSendMutexWedged = false;
  };
  window.__BUZZ_E2E_DISCONNECT_MOCK_WEBSOCKETS__ = () => {
    const socketIds = [...mockSockets.keys()];
    for (const socketId of socketIds) disconnectMockSocket(socketId);
    return socketIds.length;
  };
  // Tests flip `admitted` to exercise the denial path: mesh_ensure_client_node
  // rejects when not admitted, which proves relay membership is the gate and
  // that the create flow surfaces denial copy without spawning the agent.
  window.__BUZZ_E2E_SET_MESH__ = (mesh) => {
    if (mesh.admitted !== undefined) mockMeshState.admitted = mesh.admitted;
    if (mesh.models !== undefined) mockMeshState.models = mesh.models;
    if (mesh.denyReason !== undefined)
      mockMeshState.denyReason = mesh.denyReason;
  };
  let seedTurnSeq = Date.now();
  window.__BUZZ_E2E_SEED_ACTIVE_TURNS__ = ({
    agentPubkey,
    channelId,
    turnId,
    kind = "turn_started",
  }) => {
    seedTurnSeq += 1;
    const event = {
      seq: seedTurnSeq,
      timestamp: new Date().toISOString(),
      kind,
      agentIndex: 0,
      channelId,
      sessionId: null,
      turnId,
      payload: null,
    };
    syncAgentTurnsFromEvents(agentPubkey, [event]);
    syncAgentObserverEvents(agentPubkey, [event]);
  };
  window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ = ({ agentPubkey, events }) => {
    injectObserverEventsForE2E(agentPubkey, events);
  };
  const meshNodeStatus = (
    state: "off" | "running",
    mode: "serve" | "client" | null,
  ) => ({
    state,
    mode,
    health: { status: "ok" as const, reason: null },
    apiBaseUrl: state === "running" ? "http://127.0.0.1:9337/v1" : null,
    consoleUrl: null,
    modelId: mockMeshState.models[0]?.id ?? null,
    modelName: mockMeshState.models[0]?.name ?? null,
    inviteToken: state === "running" ? "mock-endpoint-addr" : null,
    endpointId: state === "running" ? "mock-endpoint-id" : null,
    deviceId: state === "running" ? "mock-endpoint-id" : null,
    deviceName: state === "running" ? "Mock desktop" : null,
  });
  const handleMockCommand = async (command: string, payload: unknown) => {
    const activeConfig = getConfig();
    const identity = getActiveIdentity(activeConfig);
    window.__BUZZ_E2E_COMMANDS__?.push(command);
    const loggedPayload = (() => {
      try {
        return JSON.parse(JSON.stringify(payload ?? null));
      } catch {
        return null;
      }
    })();
    window.__BUZZ_E2E_COMMAND_PAYLOADS__?.push({
      command,
      payload: loggedPayload,
    });
    window.__BUZZ_E2E_COMMAND_LOG__?.push({ command, payload });

    switch (command) {
      case "mesh_availability":
        return {
          capable: true,
          admitted: mockMeshState.admitted,
          available: mockMeshState.admitted,
          reason: mockMeshState.admitted ? null : mockMeshState.denyReason,
          models: mockMeshState.models,
          serveTargets: mockMeshState.models.map((model) => ({
            modelId: model.id,
            modelName: model.name,
            endpointAddr: "mock-endpoint-addr",
            nodeName: "Mock desktop",
            capacity: { vramGb: null },
            reporterPubkey:
              activeConfig?.mock?.meshReporterPubkey ??
              identity?.pubkey ??
              DEFAULT_MOCK_IDENTITY.pubkey,
            endpointId: "mock-endpoint-id",
            deviceId: "mock-endpoint-id",
            deviceName: "Mock desktop",
          })),
        };
      case "mesh_installed_models":
        return mockMeshState.models;
      case "mesh_node_status":
        return meshNodeStatus(mockMeshState.nodeState, mockMeshState.nodeMode);
      case "mesh_start_node": {
        const req = (
          payload as { request?: { mode?: "serve" | "client" } } | null
        )?.request;
        mockMeshState.nodeState = "running";
        mockMeshState.nodeMode = req?.mode ?? "serve";
        return meshNodeStatus(mockMeshState.nodeState, mockMeshState.nodeMode);
      }
      case "mesh_stop_node":
        mockMeshState.nodeState = "off";
        mockMeshState.nodeMode = null;
        return meshNodeStatus("off", null);
      case "mesh_ensure_client_node":
        // The invariant under test: membership is the only factor. A
        // non-admitted (non-member) caller cannot bring up the client node,
        // and no extra manual auth step exists — admission alone decides.
        if (!mockMeshState.admitted) {
          throw new Error(mockMeshState.denyReason);
        }
        mockMeshState.nodeState = "running";
        mockMeshState.nodeMode = "client";
        return meshNodeStatus("running", "client");
      case "mesh_prepare_relay_mesh_client": {
        // The Rust coordinator owns connect signaling. In the browser e2e
        // bridge, mirror the event template it would publish so specs can keep
        // asserting canonical #p targets without resurrecting the old TS
        // signaling helper.
        if (!mockMeshState.admitted) {
          throw new Error(mockMeshState.denyReason);
        }
        mockMeshState.nodeState = "running";
        mockMeshState.nodeMode = "client";
        const target = (
          payload as {
            request?: {
              target?: {
                endpointAddr?: string;
                endpointId?: string | null;
                reporterPubkey?: string;
              };
            };
          } | null
        )?.request?.target;
        const reporterPubkey = target?.reporterPubkey?.trim().toLowerCase();
        const selfPubkey = (
          identity?.pubkey ?? DEFAULT_MOCK_IDENTITY.pubkey
        ).toLowerCase();
        if (reporterPubkey && reporterPubkey !== selfPubkey) {
          window.__BUZZ_E2E_SIGNED_EVENTS__?.push({
            kind: 24621,
            tags: [["p", reporterPubkey]],
            content: JSON.stringify({
              self_endpoint_addr: "mock-endpoint-addr",
              peer_endpoint_addr: target?.endpointAddr ?? "mock-endpoint-addr",
              self_endpoint_id: "mock-endpoint-id",
              peer_endpoint_id: target?.endpointId ?? undefined,
              attempt_id: "mock-attempt-id",
            }),
          });
        }
        return meshNodeStatus("running", "client");
      }
      case "mesh_dial_endpoint_addr":
        return meshNodeStatus("running", mockMeshState.nodeMode ?? "client");
      case "mesh_status_report_payload":
        return mockMeshState.nodeState === "running"
          ? {
              token: "mock-endpoint-addr",
              node_id: "mock-endpoint-id",
              endpointId: "mock-endpoint-id",
              deviceId: "mock-endpoint-id",
              deviceName: "Mock desktop",
              hosted_models: mockMeshState.models.map((model) => model.id),
            }
          : null;
      case "mesh_agent_preset": {
        const req = (payload as { request?: { modelId?: string } } | null)
          ?.request;
        const model = req?.modelId ?? mockMeshState.models[0]?.id ?? "";
        return {
          providerId: "relay-mesh" as const,
          label: "Run on relay mesh",
          acpCommand: "",
          agentCommand: "buzz-agent",
          agentArgs: [],
          mcpCommand: "",
          model,
          envVars: {
            BUZZ_AGENT_PROVIDER: "openai",
            OPENAI_COMPAT_BASE_URL: "http://127.0.0.1:9337/v1",
            OPENAI_COMPAT_MODEL: model,
            OPENAI_COMPAT_API_KEY: "buzz-mesh-local",
            OPENAI_COMPAT_API: "chat",
          },
        };
      }
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
      case "import_identity":
        return importMockIdentity(
          (payload as { nsec?: string } | null)?.nsec ?? "",
        );
      case "apply_workspace": {
        const applyDelayMs = activeConfig?.mock?.applyWorkspaceDelayMs ?? 0;
        if (applyDelayMs > 0) {
          return new Promise((resolve) =>
            window.setTimeout(resolve, applyDelayMs),
          );
        }
        return;
      }
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
      case "get_global_notes":
        return handleGetGlobalNotes(
          payload as Parameters<typeof handleGetGlobalNotes>[0],
          activeConfig,
        );
      case "get_notes_timeline":
        return handleGetNotesTimeline(
          payload as Parameters<typeof handleGetNotesTimeline>[0],
        );
      case "get_note":
        return handleGetNote(payload as Parameters<typeof handleGetNote>[0]);
      case "get_note_reactions":
        return handleGetNoteReactions();
      case "get_liked_notes":
        return handleGetLikedNotes();
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
      case "get_project_repo_snapshot":
        return {
          latest_commit: {
            hash: "0123456789abcdef0123456789abcdef01234567",
            short_hash: "0123456",
            author_name: "Brain",
            author_email: "brain@example.com",
            timestamp: Math.floor(Date.now() / 1000) - 600,
            subject: "Add Trello board workflow details",
          },
          commits: [
            {
              hash: "0123456789abcdef0123456789abcdef01234567",
              short_hash: "0123456",
              author_name: "Brain",
              author_email: "brain@example.com",
              timestamp: Math.floor(Date.now() / 1000) - 600,
              subject: "Add Trello board workflow details",
            },
            {
              hash: "123456789abcdef0123456789abcdef012345678",
              short_hash: "1234567",
              author_name: "Thomas P",
              author_email: "thomasp@example.com",
              timestamp: Math.floor(Date.now() / 1000) - 1_800,
              subject: "Point project repository details at active branch",
            },
            {
              hash: "23456789abcdef0123456789abcdef0123456789",
              short_hash: "2345678",
              author_name: "Brain",
              author_email: "brain@example.com",
              timestamp: Math.floor(Date.now() / 1000) - 3_600,
              subject: "Make project repository-first",
            },
            {
              hash: "3456789abcdef0123456789abcdef0123456789a",
              short_hash: "3456789",
              author_name: "Git Importer",
              author_email: "git-importer@example.com",
              timestamp: Math.floor(Date.now() / 1000) - 7_200,
              subject: "Merge remote project history into local workspace",
            },
          ],
          contributors: [
            {
              name: "Brain",
              email: "brain@example.com",
              commit_count: 8,
              last_commit_at: Math.floor(Date.now() / 1000) - 600,
            },
            {
              name: "Thomas P",
              email: "thomasp@example.com",
              commit_count: 3,
              last_commit_at: Math.floor(Date.now() / 1000) - 1_800,
            },
            {
              name: "Git Importer",
              email: "git-importer@example.com",
              commit_count: 1,
              last_commit_at: Math.floor(Date.now() / 1000) - 7_200,
            },
          ],
          files: [
            {
              path: "desktop/src/features/projects/ui/ProjectDetailScreen.tsx",
              kind: "blob",
              size: 18420,
              preview_content:
                'export function ProjectDetailScreen() {\n  return <WorkspaceTabs defaultValue="files" />;\n}\n',
            },
            {
              path: "desktop/src/features/projects/ui/ProjectsView.tsx",
              kind: "blob",
              size: 16412,
              preview_content:
                "export function ProjectsView() {\n  return <ProjectsToolbar />;\n}\n",
            },
            {
              path: "desktop/src/features/projects/hooks.ts",
              kind: "blob",
              size: 9520,
              preview_content:
                "export function useProjectRepoSnapshotQuery(project) {\n  return useQuery({ queryKey: [project.id, 'repo-snapshot'] });\n}\n",
            },
            {
              path: "crates/buzz-relay/src/api/git/transport.rs",
              kind: "blob",
              size: 33120,
              preview_content:
                "// Smart HTTP git transport\n// Handles upload-pack and receive-pack for Buzz git repos.\n",
            },
          ],
        };
      case "get_project_local_repo_snapshot":
        return null;
      case "get_project_repo_sync_status":
        return {
          local_path: null,
          local_branch: null,
          local_head: null,
          local_short_head: null,
          remote_branch: "main",
          remote_head: "0123456789abcdef0123456789abcdef01234567",
          remote_short_head: "0123456",
          ahead_count: 0,
          behind_count: 0,
          has_uncommitted_changes: false,
          has_untracked_files: false,
          can_push: false,
          push_block_reason: "No local checkout found.",
        };
      case "list_project_local_repositories":
        return [];
      case "push_project_local_repository":
        return {
          pushed: true,
          message: "Pushed main to remote.",
        };
      case "get_relay_ws_url":
        return getRelayWsUrl(activeConfig);
      case "get_default_relay_url":
        return getRelayWsUrl(activeConfig);
      case "get_legacy_workspace_storage":
        return {
          workspaces: null,
          activeWorkspaceId: null,
          onboardingCompletions: [],
        };
      case "get_relay_http_url":
        return getRelayHttpUrl(activeConfig);
      case "discover_acp_providers":
        return handleDiscoverAcpRuntimes(activeConfig);
      case "install_acp_runtime":
        return handleInstallAcpRuntime(
          payload as { runtimeId?: string },
          activeConfig,
        );
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
        return handleListRelayAgents(activeConfig);
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
      case "reconcile_inbound_persona_event": {
        const nostrEvent = JSON.parse(
          (payload as { eventJson: string }).eventJson,
        ) as {
          kind: number;
          tags: string[][];
          content: string;
          created_at: number;
        };
        if (nostrEvent.kind === 30175) {
          // Persona upsert — parse content and upsert into mockPersonas by d-tag
          const dTag = nostrEvent.tags.find((t) => t[0] === "d")?.[1];
          if (dTag) {
            const content = JSON.parse(nostrEvent.content) as {
              display_name?: string;
              system_prompt?: string;
            };
            const now = new Date().toISOString();
            const existing = mockPersonas.find((p) => p.id === dTag);
            if (existing) {
              existing.display_name =
                content.display_name ?? existing.display_name;
              existing.system_prompt =
                content.system_prompt ?? existing.system_prompt;
              existing.updated_at = now;
            } else {
              mockPersonas.push({
                id: dTag,
                display_name: content.display_name ?? dTag,
                avatar_url: null,
                system_prompt: content.system_prompt ?? "",
                is_builtin: false,
                is_active: true,
                env_vars: {},
                created_at: now,
                updated_at: now,
              });
            }
          }
        } else if (nostrEvent.kind === 5) {
          // Tombstone — extract d-tag from a-tag "30175:<pubkey>:<d_tag>" and remove
          const aTagValue = nostrEvent.tags.find((t) => t[0] === "a")?.[1];
          if (aTagValue) {
            const dTag = aTagValue.split(":")[2];
            if (dTag) {
              mockPersonas = mockPersonas.filter((p) => p.id !== dTag);
            }
          }
        }
        // Mirror the real Rust backend: emit "agents-data-changed" after reconcile.
        for (const cb of tauriEventListeners.get("agents-data-changed") ?? []) {
          cb();
        }
        return undefined;
      }
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
      case "pick_team_directory":
        return handlePickTeamDirectory();
      case "install_team_from_directory":
        return handleInstallTeamFromDirectory(
          payload as Parameters<typeof handleInstallTeamFromDirectory>[0],
        );
      case "sync_team_directory":
        return handleSyncTeamDirectory(
          payload as Parameters<typeof handleSyncTeamDirectory>[0],
        );
      case "parse_team_file":
        return handleParseTeamFile();
      case "parse_persona_files":
        return handleParsePersonaFiles(
          payload as { fileBytes: number[]; fileName: string },
        );
      case "export_persona_to_json":
        return handleExportPersonaToJson(payload as { id: string });
      case "list_managed_agents":
        return handleListManagedAgents(activeConfig);
      case "get_agent_memory":
        return handleGetAgentMemory(
          (payload as Parameters<typeof handleGetAgentMemory>[0]) ?? {},
          activeConfig,
        );
      case "create_managed_agent":
        return handleCreateManagedAgent(
          payload as Parameters<typeof handleCreateManagedAgent>[0],
          activeConfig,
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
      case "discover_agent_models": {
        const input = (payload as { input?: { provider?: string } } | null)
          ?.input;
        const provider = input?.provider?.trim() ?? "";
        const openAiModels = [
          { id: "gpt-5.5", name: "GPT-5.5", description: null },
          { id: "gpt-5.4", name: "GPT-5.4", description: null },
          { id: "gpt-5.4-mini", name: "GPT-5.4 mini", description: null },
          { id: "gpt-5.4-nano", name: "GPT-5.4 nano", description: null },
        ];
        const anthropicModels = [
          {
            id: "goose-claude-4-6-opus",
            name: "Claude Opus 4.6",
            description: null,
          },
          {
            id: "goose-claude-4-6-sonnet",
            name: "Claude Sonnet 4.6",
            description: null,
          },
        ];
        const models =
          provider === "openai"
            ? openAiModels
            : provider === "anthropic"
              ? anthropicModels
              : [...anthropicModels, ...openAiModels];
        return {
          agentName: "mock-agent",
          agentVersion: "0.0.0",
          models,
          agentDefaultModel: null,
          selectedModel: null,
          supportsSwitching: true,
        };
      }
      case "get_agent_config_surface": {
        const configArgs = payload as { pubkey: string };
        return buildMockConfigSurface(configArgs.pubkey);
      }
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
          (payload as { input: Parameters<typeof handleUpdateChannel>[0] })
            .input,
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
      case "get_thread_replies":
        return handleGetThreadReplies(
          payload as Parameters<typeof handleGetThreadReplies>[0],
          activeConfig,
        );
      case "get_channel_messages_before":
        return handleGetChannelMessagesBefore(
          payload as Parameters<typeof handleGetChannelMessagesBefore>[0],
          activeConfig,
        );
      case "send_channel_message":
        return handleSendChannelMessage(
          payload as Parameters<typeof handleSendChannelMessage>[0],
          activeConfig,
        );
      case "send_managed_agent_channel_message":
        return handleSendManagedAgentChannelMessage(
          payload as Parameters<typeof handleSendManagedAgentChannelMessage>[0],
          activeConfig,
        );
      case "delete_message":
        handleDeleteMessage(
          payload as Parameters<typeof handleDeleteMessage>[0],
        );
        return null;
      case "edit_message":
        return handleEditMessage(
          payload as Parameters<typeof handleEditMessage>[0],
          activeConfig,
        );
      case "add_reaction":
        return handleAddReaction(
          payload as Parameters<typeof handleAddReaction>[0],
          activeConfig,
        );
      case "remove_reaction":
        return handleRemoveReaction(
          payload as Parameters<typeof handleRemoveReaction>[0],
          activeConfig,
        );
      case "get_media_proxy_port":
        return MOCK_MEDIA_PROXY_PORT;
      case "pick_and_upload_media":
        return await resolveMockUploadDescriptors(activeConfig);
      case "upload_media_bytes":
        return (await resolveMockUploadDescriptors(activeConfig))[0];
      case "download_image":
      case "download_file":
        // The save dialog can't run headlessly; report a successful save so the
        // FileCard / image-menu click handlers resolve. Specs assert the
        // command was invoked via `__BUZZ_E2E_COMMANDS__`, not the dialog.
        return true;
      case "get_event":
        return handleGetEvent(
          payload as Parameters<typeof handleGetEvent>[0],
          activeConfig,
        );
      case "sign_event":
        window.__BUZZ_E2E_SIGNED_EVENTS__?.push({
          content: (payload as { content: string }).content,
          kind: (payload as { kind: number }).kind,
          tags: (payload as { tags: string[][] }).tags,
        });
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
      case "nip44_encrypt_to_self":
        return (payload as { plaintext: string }).plaintext;
      case "nip44_decrypt_from_self":
        return (payload as { ciphertext: string }).ciphertext;
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
      case "plugin:window|show":
      case "plugin:window|unminimize":
      case "plugin:window|set_focus":
      case "plugin:window|set_badge_count":
      case "plugin:window|set_badge_label":
        return null;
      case "plugin:updater|check":
        return handleUpdaterCheck(activeConfig);
      case "plugin:updater|download_and_install":
        return handleUpdaterDownloadAndInstall(payload, activeConfig);
      case "relay_reconnect_hook":
        return null;
      case "relay_reconnect_hook_configured":
        return false;
      case "plugin:resources|close":
        return null;
      case "plugin:process|restart":
        return handleRestart(activeConfig);
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
        window.__BUZZ_E2E_WEBVIEW_ZOOM__ = (payload as { value: number }).value;
        return;
      case "plugin:event|listen":
        // Tauri event system (pairing, huddle) — no-op in e2e, return unlisten fn ID
        return Math.floor(Math.random() * 1_000_000);
      // ── NIP-IA identity archival ────────────────────────────────────────
      // These mocks drive the archive-button gate matrix in
      // tests/e2e/identity-archive.spec.ts. Defaults keep the button hidden
      // for non-self viewees so the negative case is the unsurprising one.
      case "resolve_oa_owner": {
        const isMe = activeConfig?.mock?.oaOwnerIsMe ?? false;
        const owner = isMe
          ? (identity?.pubkey ?? DEFAULT_MOCK_IDENTITY.pubkey)
          : "ff".repeat(32);
        return { owner, is_me: isMe };
      }
      case "list_archived_identities": {
        const archived = activeConfig?.mock?.archivedIdentities ?? [];
        return { archived };
      }
      case "archive_identity":
      case "unarchive_identity":
        // The spec only verifies UI state, not the submitted request shape;
        // returning null mirrors the Rust submit_event success path.
        return null;
      case "get_canvas": {
        const canvasReadError = activeConfig?.mock?.canvasReadError;
        if (canvasReadError) {
          throw new Error(canvasReadError);
        }
        // Return the no-canvas success shape — content null means no canvas set.
        return { content: null, updated_at: null, author: null };
      }
      default:
        throw new Error(`Unsupported mocked Tauri command: ${command}`);
    }
  };
  window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ = (command, payload) =>
    handleMockCommand(command, payload ?? null);
  mockIPC(handleMockCommand);

  // Wire up __TAURI_INTERNALS__.listen so tests can subscribe to backend-emitted
  // events (e.g. "agents-data-changed"). mockIPC already ensures __TAURI_INTERNALS__
  // exists; we just add the listen property without clobbering invoke.
  (
    window as unknown as {
      __TAURI_INTERNALS__: {
        listen?: (event: string, cb: () => void) => Promise<() => void>;
      };
    }
  ).__TAURI_INTERNALS__.listen = async (event: string, cb: () => void) => {
    let listeners = tauriEventListeners.get(event);
    if (!listeners) {
      listeners = new Set();
      tauriEventListeners.set(event, listeners);
    }
    listeners.add(cb);
    return () => {
      tauriEventListeners.get(event)?.delete(cb);
    };
  };

  installed = true;
}
