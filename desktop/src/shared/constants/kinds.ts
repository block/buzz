export const KIND_DELETION = 5;
export const KIND_REACTION = 7;
export const KIND_TEXT_NOTE = 1;
export const KIND_STREAM_MESSAGE = 9;
// Buzz-native deletion. The relay soft-deletes the target and emits a
// kind:40099 system message. Treated as a deletion marker alongside kind:5.
export const KIND_NIP29_DELETE_EVENT = 9005;
// NIP-56 report + community-moderation command kinds. Reports (1984) persist to
// the mod queue only; commands (9040–9044) are relay-validated and never stored.
// Tag shapes are pinned by buzz-sdk builders + relay moderation_commands.rs.
export const KIND_REPORT = 1984;
export const KIND_PRODUCT_FEEDBACK = 42000;
export const KIND_IA_ARCHIVE_REQUEST = 9035;
export const KIND_MODERATION_BAN = 9040;
export const KIND_MODERATION_UNBAN = 9041;
export const KIND_MODERATION_TIMEOUT = 9042;
export const KIND_MODERATION_UNTIMEOUT = 9043;
export const KIND_MODERATION_RESOLVE_REPORT = 9044;
export const KIND_STREAM_MESSAGE_V2 = 40002;
export const KIND_STREAM_MESSAGE_EDIT = 40003;
/**
 * A stream message pinned in a channel. Contract: `["h", channelId]` and one bare
 * `["e", messageId]` (no marker, so the relay never reads it as a thread reply),
 * signed by the pinner. Unpinning is a NIP-09 deletion by the pinner.
 */
export const KIND_STREAM_MESSAGE_PINNED = 40004;
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
/** Owner/admin approval request for WhatsApp Cloud API batch sends. */
export const KIND_WHATSAPP_BATCH = 47048;
/** Owner/admin decision for a WhatsApp batch row set or one agent draft. */
export const KIND_WHATSAPP_BATCH_DECISION = 47049;
/** Owner/admin immediate send or agent draft request for WhatsApp. */
export const KIND_WHATSAPP_SEND_REQUEST = 47050;
/** Owner/admin WhatsApp cap, template sync, title, and pause settings command. */
export const KIND_WHATSAPP_SETTINGS = 47051;
/** Manual WhatsApp contact stage and owner label updates. */
export const KIND_WHATSAPP_CONTACT_UPDATE = 47052;
export const KIND_APPROVAL_REQUEST = 46010;
export const KIND_MEMBER_ADDED_NOTIFICATION = 44100;
export const KIND_MEMBER_REMOVED_NOTIFICATION = 44101;
export const KIND_TYPING_INDICATOR = 20002;
export const KIND_PRESENCE_UPDATE = 20001;
export const KIND_HUDDLE_REACTION = 24810;
export const KIND_HUDDLE_STARTED = 48100;
export const KIND_HUDDLE_PARTICIPANT_JOINED = 48101;
export const KIND_HUDDLE_PARTICIPANT_LEFT = 48102;
export const KIND_HUDDLE_ENDED = 48103;
export const KIND_HUDDLE_LIVENESS = 48104;
// NIP-78 application-specific data. All use kind 30078; the relay
// differentiates them by d-tag ("read-state:<slotId>", "channel-sections",
// "channel-mutes", "channel-stars", "channel-sort", "project-sidebar-membership").
export const KIND_READ_STATE = 30078;
export const KIND_CHANNEL_SECTIONS = 30078;
export const KIND_CHANNEL_MUTES = 30078;
export const KIND_CHANNEL_STARS = 30078;
export const KIND_CHANNEL_SORT = 30078;
export const KIND_PROJECT_SIDEBAR_MEMBERSHIP = 30078;
export const KIND_COMMUNITY_THEME = 30078;
// NIP-33 persona/team/managed-agent projection events (d-tag keyed). Published
// backend-side as secrets-stripped snapshots; the inbound sync hook subscribes
// to all three to patch local records. Mirror of buzz-core's KIND_PERSONA etc.
export const KIND_PERSONA = 30175;
export const KIND_TEAM = 30176;
export const KIND_MANAGED_AGENT = 30177;
// Team catalog projection: a self-contained snapshot of a team plus every
// member's safe definition, so a recipient can rebuild it without reading the
// publisher's personas. Separate from KIND_TEAM (30176, the team's own wire
// body) so an ordinary team edit cannot disturb catalog share state.
export const KIND_TEAM_CATALOG = 30178;
export const KIND_USER_STATUS = 30315;
export const KIND_AGENT_OBSERVER_FRAME = 24200;
export const KIND_AGENT_TURN_METRIC = 44200;
export const KIND_EVENT_REMINDER = 30300;
export const KIND_REPO_ANNOUNCEMENT = 30617;
export const KIND_REPO_STATE = 30618;
// NIP-MP: project grouping above NIP-34 repositories.
export const KIND_PROJECT_ANNOUNCEMENT = 30621;
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

// Colony Phase 2 business-record kinds. Canonical NIP-33 heads are relay-signed;
// member actions and immutable versions are stored under the client channel.
export const KIND_PARTY_HEAD = 30630;
export const KIND_CLIENT_HEAD = 30631;
export const KIND_SERVICE_HEAD = 30632;
export const KIND_PROPOSAL_HEAD = 30633;
export const KIND_WORK_ITEM_HEAD = 30634;
export const KIND_KNOWLEDGE_DOCUMENT_HEAD = 30635;
export const KIND_KNOWLEDGE_FACT_HEAD = 30636;
export const KIND_SOCIAL_ACCOUNT_HEAD = 30637;
export const KIND_CONTENT_CAMPAIGN_HEAD = 30638;
export const KIND_CONTENT_POST_HEAD = 30639;
export const KIND_SITE_HEAD = 30640;
export const KIND_INVOICE_HEAD = 30641;
export const KIND_MONEY_FOLLOW_UP_HEAD = 30645;
export const KIND_PARTY_ACTION = 47000;
export const KIND_CLIENT_ACTION = 47001;
export const KIND_SERVICE_ACTION = 47002;
export const KIND_PROPOSAL_VERSION = 47003;
export const KIND_PROPOSAL_ACCEPTANCE = 47004;
export const KIND_PROPOSAL_CONVERSION_RECEIPT = 47005;
export const KIND_WORK_ITEM_ACTION = 47006;
export const KIND_DELIVERABLE_VERSION = 47007;
export const KIND_DELIVERABLE_APPROVAL = 47008;
export const KIND_KNOWLEDGE_DOCUMENT_VERSION = 47009;
export const KIND_KNOWLEDGE_FACT_VERSION = 47010;
export const KIND_KNOWLEDGE_ACCESS_CHANGE = 47011;
export const KIND_SOCIAL_ACCOUNT_AUTHORIZATION = 47012;
export const KIND_CONTENT_CAMPAIGN_ACTION = 47013;
export const KIND_CONTENT_POST_VERSION = 47014;
export const KIND_CONTENT_FEEDBACK = 47015;
export const KIND_CONTENT_APPROVAL = 47016;
export const KIND_PUBLISHING_INTENT = 47017;
export const KIND_PUBLISHING_RECEIPT = 47018;
export const KIND_SOCIAL_INBOX_ACTION = 47019;
export const KIND_SOURCED_REPORT_SNAPSHOT = 47020;
export const KIND_SITE_VERSION = 47021;
export const KIND_SITE_BUILD = 47022;
export const KIND_SITE_DEPLOYMENT = 47023;
export const KIND_SITE_DOMAIN = 47024;
export const KIND_SITE_ENQUIRY = 47025;
export const KIND_INVOICE_VERSION = 47026;
export const KIND_PAYMENT = 47027;
export const KIND_MONEY_ADJUSTMENT = 47028;
export const KIND_RECONCILIATION = 47029;
export const KIND_MONEY_FOLLOW_UP = 47030;
export const KIND_PROSPECT_ACTION = 47034;
export const KIND_PROSPECT_HEAD = 30644;
export const KIND_COMPANY_WORK_TRACKING_HEAD = 30652;
export const KIND_COMPANY_WORK_TRACKING_ACTION = 47041;

export const BUSINESS_RECORD_EVENT_KINDS = [
  KIND_PARTY_HEAD,
  KIND_CLIENT_HEAD,
  KIND_SERVICE_HEAD,
  KIND_PROSPECT_HEAD,
  KIND_PROPOSAL_HEAD,
  KIND_WORK_ITEM_HEAD,
  KIND_KNOWLEDGE_DOCUMENT_HEAD,
  KIND_KNOWLEDGE_FACT_HEAD,
  KIND_SOCIAL_ACCOUNT_HEAD,
  KIND_CONTENT_CAMPAIGN_HEAD,
  KIND_CONTENT_POST_HEAD,
  KIND_SITE_HEAD,
  KIND_INVOICE_HEAD,
  KIND_MONEY_FOLLOW_UP_HEAD,
  KIND_COMPANY_WORK_TRACKING_HEAD,
  KIND_PARTY_ACTION,
  KIND_CLIENT_ACTION,
  KIND_SERVICE_ACTION,
  KIND_PROSPECT_ACTION,
  KIND_PROPOSAL_VERSION,
  KIND_PROPOSAL_ACCEPTANCE,
  KIND_PROPOSAL_CONVERSION_RECEIPT,
  KIND_WORK_ITEM_ACTION,
  KIND_DELIVERABLE_VERSION,
  KIND_DELIVERABLE_APPROVAL,
  KIND_KNOWLEDGE_DOCUMENT_VERSION,
  KIND_KNOWLEDGE_FACT_VERSION,
  KIND_KNOWLEDGE_ACCESS_CHANGE,
  KIND_SOCIAL_ACCOUNT_AUTHORIZATION,
  KIND_CONTENT_CAMPAIGN_ACTION,
  KIND_CONTENT_POST_VERSION,
  KIND_CONTENT_FEEDBACK,
  KIND_CONTENT_APPROVAL,
  KIND_PUBLISHING_INTENT,
  KIND_PUBLISHING_RECEIPT,
  KIND_SOCIAL_INBOX_ACTION,
  KIND_SOURCED_REPORT_SNAPSHOT,
  KIND_SITE_VERSION,
  KIND_SITE_BUILD,
  KIND_SITE_DEPLOYMENT,
  KIND_SITE_DOMAIN,
  KIND_SITE_ENQUIRY,
  KIND_INVOICE_VERSION,
  KIND_PAYMENT,
  KIND_MONEY_ADJUSTMENT,
  KIND_RECONCILIATION,
  KIND_MONEY_FOLLOW_UP,
  KIND_COMPANY_WORK_TRACKING_ACTION,
] as const;

// Company records (docs/company-records.md). Goals are community-wide; asks
// live in channel threads. Heads are relay-signed. Kind 47032 (ask create)
// joins the thread timeline kinds together with its card renderer.
export const KIND_GOAL_HEAD = 30642;
export const KIND_ASK_HEAD = 30643;
export const KIND_MEMBER_POSITION_HEAD = 30648;
export const KIND_EMPLOYEE_REVISION_HEAD = 30651;
export const KIND_HIRE_HEAD = 30650;
export const KIND_EMPLOYEE_AI_ALLOWANCE_HEAD = 30653;
export const KIND_AI_SPEND_RECORD_HEAD = 30654;
export const KIND_TOOL_PERMISSION_HEAD = 30646;
export const KIND_GOAL_ACTION = 47031;
export const KIND_ASK_ACTION = 47032;
export const KIND_ASK_RESPONSE = 47033;
export const KIND_SECRET_BINDING_HEAD = 30647;
export const KIND_SECRET_BINDING_ACTION = 47036;
export const KIND_MEMBER_POSITION_ACTION = 47037;
export const KIND_EMPLOYEE_REVISION_ACTION = 47040;
export const KIND_HIRE_ACTION = 47039;
export const KIND_EMPLOYEE_AI_ALLOWANCE_ACTION = 47042;
export const KIND_AI_SPEND_RECORD_ACTION = 47043;
export const KIND_TOOL_PERMISSION_ACTION = 47035;
export const KIND_FACTORY_RUN_HEAD = 30649;
export const KIND_FACTORY_RUN_ACTION = 47038;
export const KIND_DUTY_HEAD = 30655;
export const KIND_DUTY_ACTION = 47044;
export const KIND_LESSON_HEAD = 30656;
export const KIND_LESSON_ACTION = 47045;
export const KIND_COFOUNDER_HEAD = 30670;
export const KIND_COFOUNDER_LEAD_HEAD = 30680;
export const KIND_COMPANY_FUNCTION_HEAD = 30671;
export const KIND_COFOUNDER_ACTION = 47060;
export const KIND_COMPANY_FUNCTION_ACTION = 47061;
export const KIND_WORKER_RUN_HEAD = 30672;
export const KIND_WORKER_RUN_ACTION = 47062;
export const KIND_COMPANY_ACCESS_HEAD = 30676;
export const KIND_COMPANY_ACCESS_ACTION = 47066;
export const KIND_BRIEF_HEAD = 30674;
export const KIND_ONBOARDING_HEAD = 30675;
export const KIND_BRIEF_ACTION = 47064;
export const KIND_ONBOARDING_ACTION = 47065;
export const KIND_PLAN_HEAD = 30673;
export const KIND_PLAN_ACTION = 47063;
export const KIND_PLAYBOOK_HEAD = 30677;
export const KIND_PLAYBOOK_ACTION = 47067;
export const KIND_NUMBERS_HEAD = 30678;
export const KIND_NUMBERS_ACTION = 47068;
export const KIND_CUSTOMER_HEAD = 30679;
export const KIND_CUSTOMER_ACTION = 47069;

export const COMPANY_RECORD_EVENT_KINDS = [
  KIND_GOAL_HEAD,
  KIND_ASK_HEAD,
  KIND_MEMBER_POSITION_HEAD,
  KIND_EMPLOYEE_REVISION_HEAD,
  KIND_HIRE_HEAD,
  KIND_EMPLOYEE_AI_ALLOWANCE_HEAD,
  KIND_AI_SPEND_RECORD_HEAD,
  KIND_GOAL_ACTION,
  KIND_ASK_ACTION,
  KIND_ASK_RESPONSE,
  KIND_SECRET_BINDING_HEAD,
  KIND_SECRET_BINDING_ACTION,
  KIND_MEMBER_POSITION_ACTION,
  KIND_EMPLOYEE_REVISION_ACTION,
  KIND_HIRE_ACTION,
  KIND_TOOL_PERMISSION_HEAD,
  KIND_TOOL_PERMISSION_ACTION,
  KIND_FACTORY_RUN_HEAD,
  KIND_FACTORY_RUN_ACTION,
  KIND_DUTY_HEAD,
  KIND_DUTY_ACTION,
  KIND_LESSON_HEAD,
  KIND_LESSON_ACTION,
  KIND_COFOUNDER_HEAD,
  KIND_COMPANY_FUNCTION_HEAD,
  KIND_COFOUNDER_ACTION,
  KIND_COMPANY_FUNCTION_ACTION,
  KIND_EMPLOYEE_AI_ALLOWANCE_ACTION,
  KIND_AI_SPEND_RECORD_ACTION,
  KIND_WORKER_RUN_HEAD,
  KIND_WORKER_RUN_ACTION,
  KIND_COMPANY_ACCESS_HEAD,
  KIND_COMPANY_ACCESS_ACTION,
  KIND_BRIEF_HEAD,
  KIND_BRIEF_ACTION,
  KIND_ONBOARDING_HEAD,
  KIND_ONBOARDING_ACTION,
  KIND_PLAN_HEAD,
  KIND_PLAYBOOK_HEAD,
  KIND_PLAN_ACTION,
  KIND_PLAYBOOK_ACTION,
] as const;

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
  KIND_ASK_ACTION,
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
  KIND_ASK_ACTION, // 47032, ask create card in its thread
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
