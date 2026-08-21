import {
  channelTranscript,
  channelWindow,
  threadMessages,
} from "./store.mjs";

/** Messages of channel context handed to stage 1 for an untagged message. */
export const CHANNEL_CONTEXT_DEPTH = 12;

/** Window used for "a lot of messages in a short time span". */
export const VELOCITY_WINDOW_SECONDS = 30 * 60;

/** A thread whose last message is this recent still counts as ongoing. */
export const ONGOING_SECONDS = 60 * 60;

export const FOLLOW_UP_PATTERN =
  /^\s*(?:bump|ping|any update|any updates|any news|still blocked|still waiting|following up|follow up|gentle reminder|reminder)\b/i;

export const LOW_VALUE_PATTERN =
  /^(?:\+1|ty|thx|thanks|thank you|nice|cool|lol|haha|ok|okay|k|got it|sounds good|congrats|welcome|gm|good morning|morning|hi|hey|hello|yep|yes|no|done|same|this|ditto|👍|🎉|✅)[\s!.?]*$/i;

const INCIDENT_CHANNEL_PATTERN = /incident|outage|sev\d|oncall|on-call|pager/i;

export function isFollowUp(content) {
  return FOLLOW_UP_PATTERN.test(String(content ?? "").trim());
}

export function isLowValue(content) {
  return LOW_VALUE_PATTERN.test(String(content ?? "").trim());
}

export function isIncidentChannel(channelName) {
  return INCIDENT_CHANNEL_PATTERN.test(String(channelName ?? ""));
}

/**
 * Context stage 1 needs to judge one message. A message that tags the viewer
 * inside a thread gets the whole thread; anything else gets the recent
 * channel window.
 */
export function messageContext(pubkey, message) {
  const channelId = message?.channelId;
  if (!channelId) return { scope: "none", messages: [], engagement: 0 };

  const thread = message.threadRootId
    ? threadMessages(pubkey, channelId, message.threadRootId)
    : [];
  const engagement = thread.length > 0 ? engagementScore(velocityOf(thread)) : 0;

  if (message.isMention && thread.length > 0) {
    return {
      scope: "thread",
      messages: thread.filter((row) => row.eventId !== message.eventId),
      engagement,
    };
  }

  return {
    scope: "channel",
    messages: channelWindow(
      pubkey,
      channelId,
      message.eventId,
      CHANNEL_CONTEXT_DEPTH,
    ),
    engagement,
  };
}

function conversationRows(pubkey, fibre) {
  const channelId = fibre.channelId ?? fibre.artifacts?.[0]?.channelId ?? null;
  if (!channelId) return [];

  const roots = new Set();
  for (const artifact of fibre.artifacts ?? []) {
    roots.add(artifact.threadRootId ?? artifact.eventId);
  }

  const byEventId = new Map();
  for (const root of roots) {
    for (const row of threadMessages(pubkey, channelId, root)) {
      byEventId.set(row.eventId, row);
    }
  }

  // A fibre whose messages predate the transcript still has its artifacts.
  for (const artifact of fibre.artifacts ?? []) {
    if (byEventId.has(artifact.eventId)) continue;
    byEventId.set(artifact.eventId, {
      eventId: artifact.eventId,
      threadRootId: artifact.threadRootId ?? null,
      authorPubkey: artifact.authorPubkey ?? null,
      authorLabel: artifact.authorLabel ?? null,
      content: artifact.content ?? "",
      createdAt: artifact.createdAt ?? 0,
      isMention: false,
      isSelf: false,
    });
  }

  return [...byEventId.values()].sort((a, b) => a.createdAt - b.createdAt);
}

function velocityOf(rows) {
  if (rows.length === 0) return { messages: 0, participants: 0 };
  const latest = rows[rows.length - 1].createdAt;
  const window = rows.filter(
    (row) => row.createdAt > latest - VELOCITY_WINDOW_SECONDS,
  );
  const participants = new Set(
    window.map((row) => row.authorPubkey).filter(Boolean),
  );
  return { messages: window.length, participants: participants.size };
}

/**
 * How much of a live discussion this is, independent of whether it needs
 * anything from the viewer. Deterministic so the "hot" lane never depends on
 * a model call.
 */
export function engagementScore({ messages, participants }) {
  const byVolume = Math.min(60, messages * 6);
  const byPeople = Math.min(40, Math.max(0, participants - 1) * 12);
  return Math.max(0, Math.min(100, byVolume + byPeople));
}

/**
 * Facts about an accumulated fibre, derived from the transcript. Stage 2
 * reads these as prompt input, and `scoreAdjustments` turns the ones we do
 * not want to leave to model judgment into a guaranteed score delta.
 */
export function fibreSignals(pubkey, fibre, now = Math.floor(Date.now() / 1000)) {
  const channelId = fibre.channelId ?? fibre.artifacts?.[0]?.channelId ?? null;
  const rows = conversationRows(pubkey, fibre);
  const artifacts = fibre.artifacts ?? [];

  const lastSelfAt = rows.reduce(
    (latest, row) => (row.isSelf ? Math.max(latest, row.createdAt) : latest),
    Number.NEGATIVE_INFINITY,
  );
  const unansweredPings = rows.filter(
    (row) => row.isMention && row.createdAt > lastSelfAt,
  ).length;

  const earliestArtifactAt = artifacts.reduce(
    (earliest, artifact) => Math.min(earliest, artifact.createdAt ?? Infinity),
    Infinity,
  );
  const involvedInThread = rows.some(
    (row) => row.isSelf && row.createdAt < earliestArtifactAt,
  );
  const involvedInChannel = channelId
    ? channelTranscript(pubkey, channelId).some(
        (row) => row.isSelf && row.createdAt < earliestArtifactAt,
      )
    : false;

  const velocity = velocityOf(rows);
  const latestAt = rows.length > 0 ? rows[rows.length - 1].createdAt : 0;
  const incidentChannel = isIncidentChannel(fibre.channelName);

  return {
    conversationSize: rows.length,
    mentionCount: rows.filter((row) => row.isMention).length,
    unansweredPings,
    hasReplyFromViewer: lastSelfAt !== Number.NEGATIVE_INFINITY,
    velocity,
    engagement: engagementScore(velocity),
    pastInvolvement: {
      thread: involvedInThread,
      channel: involvedInChannel,
      any: involvedInThread || involvedInChannel,
    },
    incidentChannel,
    incidentOngoing: incidentChannel && now - latestAt <= ONGOING_SECONDS,
    followUpOnly:
      artifacts.length > 0 &&
      artifacts.every((artifact) => isFollowUp(artifact.content)),
    isDm: Boolean(fibre.isDm),
  };
}

/**
 * Score movements we guarantee rather than leave to the model. Returned in
 * the same `{ weight, label }` shape the detail pane already renders.
 */
export function scoreAdjustments(signals) {
  const entries = [];

  if (signals.unansweredPings > 1) {
    const delta = Math.min(25, signals.unansweredPings * 10);
    entries.push({
      weight: `+${delta}`,
      label: `Pinged ${signals.unansweredPings} times with no reply from you`,
    });
  }
  if (signals.followUpOnly) {
    entries.push({ weight: "-15", label: "Follow-up with no new information" });
  }
  if (signals.pastInvolvement.any) {
    entries.push({ weight: "+10", label: "You have worked on this before" });
  }
  if (signals.incidentOngoing) {
    entries.push({ weight: "+25", label: "Ongoing incident channel" });
  }

  const delta = entries.reduce(
    (total, entry) => total + Number(entry.weight),
    0,
  );
  return { delta, entries };
}
