export const KIND_DELETION = 5;
export const KIND_REACTION = 7;
export const KIND_TEXT_NOTE = 1;
export const KIND_STREAM_MESSAGE = 9;
// Buzz-native deletion. The relay soft-deletes the target and emits a
// kind:40099 system message. Treated as a deletion marker alongside kind:5.
export const KIND_NIP29_DELETE_EVENT = 9005;
export const KIND_STREAM_MESSAGE_V2 = 40002;
export const KIND_STREAM_MESSAGE_EDIT = 40003;
export const KIND_CHANNEL_THREAD_SUMMARY = 39005;
export const KIND_CHANNEL_WINDOW_BOUNDS = 39006;
export const KIND_STREAM_MESSAGE_DIFF = 40008;
export const KIND_REMINDER = 40007;
export const KIND_SYSTEM_MESSAGE = 40099;
export const KIND_JOB_REQUEST = 43001;
export const KIND_JOB_ACCEPTED = 43002;
export const KIND_JOB_PROGRESS = 43003;
export const KIND_JOB_RESULT = 43004;
export const KIND_JOB_CANCEL = 43005;
export const KIND_JOB_ERROR = 43006;
export const KIND_FORUM_POST = 45001;
export const KIND_FORUM_COMMENT = 45003;
export const KIND_APPROVAL_REQUEST = 46010;
export const KIND_MEMBER_ADDED_NOTIFICATION = 44100;
export const KIND_MEMBER_REMOVED_NOTIFICATION = 44101;
export const KIND_TYPING_INDICATOR = 20002;
export const KIND_HUDDLE_REACTION = 24810;
export const KIND_HUDDLE_STARTED = 48100;
export const KIND_HUDDLE_PARTICIPANT_JOINED = 48101;
export const KIND_HUDDLE_PARTICIPANT_LEFT = 48102;
export const KIND_HUDDLE_ENDED = 48103;
// NIP-78 application-specific data. All use kind 30078; the relay
// differentiates them by d-tag ("read-state:<slotId>", "channel-sections", "channel-mutes", "channel-stars", "channel-sort").
export const KIND_READ_STATE = 30078;
export const KIND_CHANNEL_SECTIONS = 30078;
export const KIND_CHANNEL_MUTES = 30078;
export const KIND_CHANNEL_STARS = 30078;
export const KIND_CHANNEL_SORT = 30078;
// NIP-33 persona/team/managed-agent projection events (d-tag keyed). Published
// backend-side as secrets-stripped snapshots; the inbound sync hook subscribes
// to all three to patch local records. Mirror of buzz-core's KIND_PERSONA etc.
export const KIND_PERSONA = 30175;
export const KIND_TEAM = 30176;
export const KIND_MANAGED_AGENT = 30177;
export const KIND_USER_STATUS = 30315;
export const KIND_AGENT_OBSERVER_FRAME = 24200;
export const KIND_AGENT_TURN_METRIC = 44200;
export const KIND_MESH_STATUS_REPORT = 24620;
export const KIND_MESH_CONNECT_REQUEST = 24621;
export const KIND_MESH_CALL_ME_NOW = 24622;
export const KIND_EVENT_REMINDER = 30300;
export const KIND_REPO_ANNOUNCEMENT = 30617;
export const KIND_REPO_STATE = 30618;
export const KIND_GIT_PATCH = 1617;
export const KIND_GIT_PULL_REQUEST = 1618;
export const KIND_GIT_PR_UPDATE = 1619;
export const KIND_GIT_ISSUE = 1621;
export const KIND_GIT_STATUS_OPEN = 1630;
export const KIND_GIT_STATUS_MERGED = 1631;
export const KIND_GIT_STATUS_CLOSED = 1632;
export const KIND_GIT_STATUS_DRAFT = 1633;
// NIP-DV: relay-signed per-viewer DM visibility snapshot (d=viewer pubkey,
// h-tags = currently-hidden DM channel ids).
export const KIND_DM_VISIBILITY = 30622;
export const KIND_CHAT_METADATA = 30623;

// Human-visible "new content" message kinds. Used as the unread trigger set
// (sidebar badges, catch-up queries) and as the Home-feed mention query.
// Reactions, edits, diffs, deletions, and system messages are deliberately
// excluded: they can land after the last human-visible message and would
// otherwise create phantom unreads.
export const CHANNEL_MESSAGE_EVENT_KINDS = [
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
  KIND_FORUM_POST,
  KIND_FORUM_COMMENT,
] as const;

// Keep this in sync with the Home-feed mention query in buzz-db.
export const HOME_MENTION_EVENT_KINDS = [...CHANNEL_MESSAGE_EVENT_KINDS];

export const CHANNEL_EVENT_KINDS = [
  KIND_DELETION, // 5 — NIP-09 event deletions
  KIND_REACTION, // 7 — NIP-25 reactions
  KIND_NIP29_DELETE_EVENT, // 9005 — NIP-29 / Buzz-native deletions
  ...CHANNEL_MESSAGE_EVENT_KINDS,
  40001, // legacy: pre-migration stream messages
  KIND_STREAM_MESSAGE_EDIT, // 40003 — message edits
  KIND_STREAM_MESSAGE_DIFF, // 40008 — message diffs
  KIND_SYSTEM_MESSAGE, // 40099 — system messages (join, leave, etc.)
  KIND_HUDDLE_STARTED, // 48100 — visible huddle session card
  KIND_HUDDLE_PARTICIPANT_JOINED, // 48101 — huddle lifecycle overlay
  KIND_HUDDLE_PARTICIPANT_LEFT, // 48102 — huddle lifecycle overlay
  KIND_HUDDLE_ENDED, // 48103 — huddle lifecycle overlay
] as const;

// Auxiliary (non-row) timeline kinds: events that overlay onto or hide an
// existing message rather than rendering their own row — reactions, edits, and
// deletions. History fetches request the visible content kinds only, so the
// `limit` budget buys visible message depth instead of being diluted by these
// (on a reaction-heavy channel a 200-event window was only ~136 messages).
// They are backfilled separately by `#e` reference over the loaded message ids
// — by reference, not by time window, so a late edit/delete for a visible old
// message still applies. NOTE: kind:40008 (diff) renders its OWN row, so it is
// a content kind, not aux.
export const CHANNEL_AUX_EVENT_KINDS = [
  KIND_DELETION, // 5 — NIP-09 event deletions
  KIND_REACTION, // 7 — NIP-25 reactions
  KIND_NIP29_DELETE_EVENT, // 9005 — NIP-29 / Buzz-native deletions
  KIND_STREAM_MESSAGE_EDIT, // 40003 — message edits
] as const;

// Visible content kinds the main timeline renders as their own rows. Mirrors
// `isTimelineContentEvent` in formatTimelineMessages.ts — keep the two in sync.
// This is the kind set the history fetch requests so the `limit` budget maps
// to visible rows; auxiliary overlays (CHANNEL_AUX_EVENT_KINDS) are fetched
// separately by `#e` reference. Forum kinds (45001/45003) are excluded: forum
// channels use a different query path, not this timeline.
export const CHANNEL_TIMELINE_CONTENT_KINDS = [
  KIND_STREAM_MESSAGE, // 9
  KIND_STREAM_MESSAGE_V2, // 40002
  KIND_STREAM_MESSAGE_DIFF, // 40008 — diff messages (own row)
  KIND_SYSTEM_MESSAGE, // 40099 — system rows (join/leave/channel-created)
  KIND_JOB_REQUEST, // 43001
  KIND_JOB_ACCEPTED, // 43002
  KIND_JOB_PROGRESS, // 43003
  KIND_JOB_RESULT, // 43004
  KIND_JOB_CANCEL, // 43005
  KIND_JOB_ERROR, // 43006
  KIND_HUDDLE_STARTED, // 48100 — huddle session card
] as const;

// Timeline kinds that are NOT conversational: relay-signed system rows
// (channel-created, member-joined) and job-lifecycle events. These render in
// the timeline but must not count toward the channel's unread pill — a freshly
// created channel carries one channel_created + N member_joined system rows
// that would otherwise show as phantom unreads ("4 unread, 1 message").
const NON_CONVERSATIONAL_UNREAD_KINDS: ReadonlySet<number> = new Set([
  KIND_SYSTEM_MESSAGE, // 40099
  KIND_JOB_REQUEST, // 43001
  KIND_JOB_ACCEPTED, // 43002
  KIND_JOB_PROGRESS, // 43003
  KIND_JOB_RESULT, // 43004
  KIND_JOB_CANCEL, // 43005
  KIND_JOB_ERROR, // 43006
  KIND_HUDDLE_STARTED, // 48100 — huddle cards are visible but non-conversational
  KIND_HUDDLE_PARTICIPANT_JOINED, // 48101
  KIND_HUDDLE_PARTICIPANT_LEFT, // 48102
  KIND_HUDDLE_ENDED, // 48103
]);

// Whether a timeline message kind should count toward unread tallies. An
// undefined kind (optimistic/pending rows whose kind has not populated) is
// treated as conversational so a legitimately unread message is never dropped.
export function isConversationalUnreadKind(kind: number | undefined): boolean {
  return kind === undefined || !NON_CONVERSATIONAL_UNREAD_KINDS.has(kind);
}
