import type {
  Channel,
  ChannelMember,
  RelayEvent,
  RespondToMode,
} from "@/shared/api/types";

import type {
  TimelineMessage,
  TimelineReaction,
} from "@/features/messages/types";
import { getThreadReference } from "@/features/messages/lib/threading";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import {
  KIND_JOB_ACCEPTED,
  KIND_JOB_CANCEL,
  KIND_JOB_ERROR,
  KIND_JOB_PROGRESS,
  KIND_JOB_REQUEST,
  KIND_JOB_RESULT,
  KIND_DELETION,
  KIND_REACTION,
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
  KIND_STREAM_MESSAGE_EDIT,
  KIND_STREAM_MESSAGE_DIFF,
  KIND_SYSTEM_MESSAGE,
} from "@/shared/constants/kinds";
import { resolveEventAuthorPubkey } from "@/shared/lib/authors";
import { formatTime } from "@/features/messages/lib/dateFormatters";
// Pure overlay helper lives in a sibling .mjs so node:test (no TS loader)
// can exercise the exact same source the renderer uses.
import { applyEditTagOverlay } from "@/features/messages/lib/applyEditTagOverlay.mjs";

const HEX_RE = /^[0-9a-f]+$/i;

function isTimelineContentEvent(event: RelayEvent) {
  return (
    event.kind === KIND_STREAM_MESSAGE ||
    event.kind === KIND_STREAM_MESSAGE_V2 ||
    event.kind === KIND_STREAM_MESSAGE_DIFF ||
    event.kind === KIND_SYSTEM_MESSAGE ||
    event.kind === KIND_JOB_REQUEST ||
    event.kind === KIND_JOB_ACCEPTED ||
    event.kind === KIND_JOB_PROGRESS ||
    event.kind === KIND_JOB_RESULT ||
    event.kind === KIND_JOB_CANCEL ||
    event.kind === KIND_JOB_ERROR
  );
}

function getDeletionTargets(tags: string[][]) {
  return tags
    .filter(
      (tag) =>
        tag[0] === "e" &&
        typeof tag[1] === "string" &&
        tag[1].length === 64 &&
        HEX_RE.test(tag[1]),
    )
    .map((tag) => tag[1]);
}

function getReactionTargetId(tags: string[][]) {
  for (let index = tags.length - 1; index >= 0; index -= 1) {
    const tag = tags[index];
    if (
      tag?.[0] === "e" &&
      typeof tag[1] === "string" &&
      tag[1].length === 64 &&
      HEX_RE.test(tag[1])
    ) {
      return tag[1];
    }
  }

  return null;
}

function formatMessageAuthor(
  event: RelayEvent,
  channel: Channel | null,
  currentPubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
) {
  const authorPubkey = resolveEventAuthorPubkey({
    pubkey: event.pubkey,
    tags: event.tags,
    preferActorTag: true,
    requireChannelTagForPTags: true,
  });
  const fallbackName =
    channel?.channelType === "dm"
      ? (() => {
          const participantIndex =
            channel.participantPubkeys.indexOf(authorPubkey);
          if (participantIndex < 0) {
            return null;
          }

          return channel.participants[participantIndex] ?? null;
        })()
      : null;

  return resolveUserLabel({
    pubkey: authorPubkey,
    currentPubkey,
    fallbackName,
    profiles,
    preferResolvedSelfLabel: true,
  });
}

function getAuthorAvatarUrl(input: {
  authorPubkey: string;
  currentPubkey: string | undefined;
  currentUserAvatarUrl: string | null;
  profiles: UserProfileLookup | undefined;
}) {
  const { authorPubkey, currentPubkey, currentUserAvatarUrl, profiles } = input;

  if (currentPubkey === authorPubkey) {
    return currentUserAvatarUrl ?? null;
  }

  return profiles?.[authorPubkey.toLowerCase()]?.avatarUrl ?? null;
}

export function formatTimelineMessages(
  events: RelayEvent[],
  channel: Channel | null,
  currentPubkey: string | undefined,
  currentUserAvatarUrl: string | null,
  profiles?: UserProfileLookup,
  members?: ChannelMember[],
  /** Map from lowercase pubkey → persona display name for bot messages. */
  personaLookup?: Map<string, string>,
  /** Map from lowercase pubkey → respond-to mode for bot messages. */
  respondToLookup?: Map<string, RespondToMode>,
): TimelineMessage[] {
  const currentPubkeyLower = currentPubkey?.toLowerCase();
  const roleByPubkey = new Map<string, string>();
  if (members) {
    for (const member of members) {
      roleByPubkey.set(member.pubkey.toLowerCase(), member.role);
    }
  }
  const deletedEventIds = new Set<string>();
  for (const event of events) {
    if (event.kind !== KIND_DELETION) {
      continue;
    }

    for (const targetId of getDeletionTargets(event.tags)) {
      deletedEventIds.add(targetId);
    }
  }

  // Build a map of latest edit per original message: targetId → { content, tags, createdAt }.
  // When multiple edits exist for the same message, the most recent one wins.
  // The edit's own tags are kept so the renderer can overlay imeta tags
  // (attachments) from the edit onto the original event — non-imeta tags on
  // the original (`h`, `p` mentions, etc.) stay untouched.
  const editsByTargetId = new Map<
    string,
    { content: string; tags: string[][]; createdAt: number }
  >();
  for (const event of events) {
    if (
      event.kind !== KIND_STREAM_MESSAGE_EDIT ||
      deletedEventIds.has(event.id)
    ) {
      continue;
    }

    const targetId = getReactionTargetId(event.tags);
    if (!targetId || deletedEventIds.has(targetId)) {
      continue;
    }

    const existing = editsByTargetId.get(targetId);
    if (!existing || event.created_at > existing.createdAt) {
      editsByTargetId.set(targetId, {
        content: event.content,
        tags: event.tags,
        createdAt: event.created_at,
      });
    }
  }

  const visibleEvents = events.filter(
    (event) => isTimelineContentEvent(event) && !deletedEventIds.has(event.id),
  );
  const eventsById = new Map(visibleEvents.map((event) => [event.id, event]));
  const reactionPresence = new Map<
    string,
    {
      targetId: string;
      actorPubkey: string;
      emoji: string;
      emojiUrl?: string;
    }
  >();

  for (const event of events) {
    if (event.kind !== KIND_REACTION || deletedEventIds.has(event.id)) {
      continue;
    }

    const targetId = getReactionTargetId(event.tags);
    if (!targetId || deletedEventIds.has(targetId)) {
      continue;
    }

    const actorPubkey = resolveEventAuthorPubkey({
      pubkey: event.pubkey,
      tags: event.tags,
      preferActorTag: true,
      requireChannelTagForPTags: true,
    }).toLowerCase();
    const emoji = event.content.trim() || "+";
    // Custom-emoji reaction (NIP-30): content is `:shortcode:` and the URL
    // rides on a matching `["emoji", shortcode, url]` tag.
    let emojiUrl: string | undefined;
    if (emoji.startsWith(":") && emoji.endsWith(":")) {
      const shortcode = emoji.slice(1, -1);
      emojiUrl = event.tags.find(
        (t) => t[0] === "emoji" && t[1] === shortcode && t[2],
      )?.[2];
    }
    reactionPresence.set(`${targetId}:${actorPubkey}:${emoji}`, {
      targetId,
      actorPubkey,
      emoji,
      emojiUrl,
    });
  }

  const reactionsByEventId = new Map<string, Map<string, TimelineReaction>>();
  for (const {
    targetId,
    actorPubkey,
    emoji,
    emojiUrl,
  } of reactionPresence.values()) {
    const current = reactionsByEventId.get(targetId) ?? new Map();
    const existing = current.get(emoji) ?? {
      emoji,
      emojiUrl,
      count: 0,
      reactedByCurrentUser: false,
      users: [],
    };

    existing.count += 1;
    if (currentPubkeyLower && actorPubkey === currentPubkeyLower) {
      existing.reactedByCurrentUser = true;
    }

    const profile = profiles?.[actorPubkey];
    const displayName =
      currentPubkeyLower && actorPubkey === currentPubkeyLower
        ? "You"
        : profile?.displayName?.trim() ||
          profile?.nip05Handle?.trim() ||
          `${actorPubkey.slice(0, 8)}…`;
    existing.users.push({
      pubkey: actorPubkey,
      displayName,
      avatarUrl: profile?.avatarUrl ?? null,
    });

    current.set(emoji, existing);
    reactionsByEventId.set(targetId, current);
  }

  const authorPubkeyByEventId = new Map<string, string>();
  const authorLabelByEventId = new Map<string, string>();
  const depthByEventId = new Map<string, number>();
  const resolvingEventIds = new Set<string>();

  function getAuthorLabel(event: RelayEvent) {
    const cached = authorLabelByEventId.get(event.id);
    if (cached) {
      return cached;
    }

    const authorPubkey = resolveEventAuthorPubkey({
      pubkey: event.pubkey,
      tags: event.tags,
      preferActorTag: true,
      requireChannelTagForPTags: true,
    });
    const author = formatMessageAuthor(event, channel, currentPubkey, profiles);

    authorPubkeyByEventId.set(event.id, authorPubkey);
    authorLabelByEventId.set(event.id, author);
    return author;
  }

  function getDepth(event: RelayEvent): number {
    const cached = depthByEventId.get(event.id);
    if (cached !== undefined) {
      return cached;
    }

    if (resolvingEventIds.has(event.id)) {
      return 0;
    }

    const thread = getThreadReference(event.tags);
    if (!thread.parentId) {
      depthByEventId.set(event.id, 0);
      return 0;
    }

    const parent = eventsById.get(thread.parentId);
    if (!parent) {
      const fallbackDepth =
        thread.rootId && thread.rootId !== thread.parentId ? 2 : 1;
      depthByEventId.set(event.id, fallbackDepth);
      return fallbackDepth;
    }

    resolvingEventIds.add(event.id);
    const depth = getDepth(parent) + 1;
    resolvingEventIds.delete(event.id);
    depthByEventId.set(event.id, depth);
    return depth;
  }

  return visibleEvents.map((event) => {
    const author = getAuthorLabel(event);
    const authorPubkey =
      authorPubkeyByEventId.get(event.id) ??
      resolveEventAuthorPubkey({
        pubkey: event.pubkey,
        tags: event.tags,
        preferActorTag: true,
        requireChannelTagForPTags: true,
      });
    const thread = getThreadReference(event.tags);
    const edit = editsByTargetId.get(event.id);
    const role = roleByPubkey.get(authorPubkey.toLowerCase());
    return {
      id: event.id,
      createdAt: event.created_at,
      pubkey: authorPubkey,
      author,
      avatarUrl: getAuthorAvatarUrl({
        authorPubkey,
        currentPubkey,
        currentUserAvatarUrl,
        profiles,
      }),
      role,
      personaDisplayName:
        role === "bot"
          ? personaLookup?.get(authorPubkey.toLowerCase())
          : undefined,
      respondTo:
        role === "bot"
          ? respondToLookup?.get(authorPubkey.toLowerCase())
          : undefined,
      time: formatTime(event.created_at),
      body: edit ? edit.content : event.content,
      parentId: thread.parentId,
      rootId: thread.rootId,
      depth: getDepth(event),
      accent: currentPubkey === authorPubkey,
      pending: event.pending,
      edited: edit !== undefined,
      kind: event.kind,
      // When edited, swap the original event's imeta tags for the edit's
      // imeta tags. All non-imeta tags on the original are preserved.
      // Logic lives in `applyEditTagOverlay.mjs` so prod and tests share
      // a single source.
      tags: applyEditTagOverlay(event.tags, edit?.tags),
      reactions: (() => {
        const reactions = reactionsByEventId.get(event.id);
        return reactions ? [...reactions.values()] : undefined;
      })(),
    };
  });
}

function extractSystemMessagePubkeys(event: RelayEvent): string[] {
  if (event.kind !== KIND_SYSTEM_MESSAGE) {
    return [];
  }

  try {
    const payload = JSON.parse(event.content);
    const pubkeys: string[] = [];
    if (typeof payload.actor === "string") {
      pubkeys.push(payload.actor.toLowerCase());
    }
    if (typeof payload.target === "string") {
      pubkeys.push(payload.target.toLowerCase());
    }
    return pubkeys;
  } catch {
    return [];
  }
}

export function collectMessageAuthorPubkeys(events: RelayEvent[]) {
  const pubkeys = new Set<string>();

  for (const event of events) {
    if (!isTimelineContentEvent(event)) {
      continue;
    }

    if (event.kind === KIND_SYSTEM_MESSAGE) {
      for (const pk of extractSystemMessagePubkeys(event)) {
        pubkeys.add(pk);
      }
    } else {
      pubkeys.add(
        resolveEventAuthorPubkey({
          pubkey: event.pubkey,
          tags: event.tags,
          preferActorTag: true,
          requireChannelTagForPTags: true,
        }).toLowerCase(),
      );
    }
  }

  return [...pubkeys];
}
