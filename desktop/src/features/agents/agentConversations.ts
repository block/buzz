import type { TimelineMessage } from "@/features/messages/types";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { Channel, RelayEvent } from "@/shared/api/types";
import {
  KIND_AGENT_CONVERSATION,
  KIND_AGENT_CONVERSATION_COMPAT,
} from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  collectConversationContextMessages,
  deriveTitleFromContext,
} from "./agentConversationTitles";

export { buildAgentConversationRecap } from "./agentConversationRecap";
export { deriveAgentConversationTitle } from "./agentConversationTitles";

const HIDDEN_AGENT_CONVERSATIONS_STORAGE_PREFIX =
  "buzz-hidden-agent-conversations.v1";
const AGENT_CONVERSATIONS_STORAGE_PREFIX = "buzz-agent-conversations.v1";
const MAX_PERSISTED_AGENT_CONVERSATIONS = 100;
export type AgentConversationTitleStatus = "provisional" | "resolved";

export type AgentConversation = {
  id: string;
  agentName: string;
  agentPubkey: string;
  agentReply: TimelineMessage;
  channelId: string;
  channelName: string;
  contextMessages: TimelineMessage[];
  createdAt: number;
  parentMessage: TimelineMessage | null;
  threadRootId: string;
  threadRootMessage: TimelineMessage | null;
  title: string;
  titleStatus: AgentConversationTitleStatus;
};

export type OpenAgentConversationInput = {
  agentName: string;
  agentPubkey: string;
  agentReply: TimelineMessage;
  channel: Pick<Channel, "id" | "name">;
  contextMessages?: TimelineMessage[];
  parentMessage: TimelineMessage | null;
  threadRootMessage: TimelineMessage | null;
};

export type AgentConversationMarker = {
  agentName: string;
  agentPubkey: string;
  agentReplyId: string;
  channelId: string;
  createdAt: number;
  eventId: string;
  parentMessageId: string | null;
  startedAt: number;
  starterPubkey: string;
  summary: string | null;
  summaryAuthorName: string | null;
  summaryAuthorPubkey: string | null;
  summaryCreatedAt: number | null;
  threadRootMessageId: string | null;
  threadRootId: string;
  title: string;
  titleStatus: AgentConversationTitleStatus;
};

export type AgentConversationMarkerUpdate = {
  summary?: string | null;
  summaryAuthorName?: string | null;
  summaryAuthorPubkey?: string | null;
  summaryCreatedAt?: number | null;
};

export type AgentConversationRecapInput = {
  agentPubkeys: ReadonlySet<string> | readonly string[];
  conversationTitle?: string | null;
  messages: readonly TimelineMessage[];
};

export type AgentConversationRouteableParticipant = {
  canMessage: boolean;
  pubkey: string;
};

export function hiddenAgentConversationsStorageKey(pubkey: string): string {
  return `${HIDDEN_AGENT_CONVERSATIONS_STORAGE_PREFIX}:${pubkey}`;
}

export function agentConversationsStorageKey(pubkey: string): string {
  return `${AGENT_CONVERSATIONS_STORAGE_PREFIX}:${pubkey}`;
}

export function getAutoRoutedAgentConversationPubkeys(
  participants: readonly AgentConversationRouteableParticipant[],
): string[] {
  if (participants.length !== 1) {
    return [];
  }

  const [participant] = participants;
  return participant.canMessage ? [participant.pubkey] : [];
}

export function buildAgentConversationMentionPubkeys({
  autoRouteAgentPubkeys,
  mentionPubkeys,
}: {
  autoRouteAgentPubkeys: readonly string[];
  mentionPubkeys: readonly string[];
}): string[] {
  const seenPubkeys = new Set<string>();
  const merged: string[] = [];
  const add = (pubkey: string) => {
    const normalized = normalizePubkey(pubkey);
    if (!normalized || seenPubkeys.has(normalized)) {
      return;
    }

    seenPubkeys.add(normalized);
    merged.push(pubkey);
  };

  for (const pubkey of autoRouteAgentPubkeys) {
    add(pubkey);
  }
  for (const pubkey of mentionPubkeys) {
    add(pubkey);
  }

  return merged;
}

export function readHiddenAgentConversationIds(pubkey: string): Set<string> {
  try {
    const raw = window.localStorage.getItem(
      hiddenAgentConversationsStorageKey(pubkey),
    );
    if (!raw) {
      return new Set();
    }

    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return new Set();
    }

    return new Set(
      parsed.filter((value): value is string => typeof value === "string"),
    );
  } catch {
    return new Set();
  }
}

export function writeHiddenAgentConversationIds(
  pubkey: string,
  ids: ReadonlySet<string>,
): void {
  try {
    window.localStorage.setItem(
      hiddenAgentConversationsStorageKey(pubkey),
      JSON.stringify([...ids]),
    );
  } catch {
    // Best-effort local preference; ignore storage failures.
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function maybeString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function maybeNullableString(value: unknown): string | null | undefined {
  if (value === null) {
    return null;
  }

  return maybeString(value);
}

function parseStoredTimelineMessage(value: unknown): TimelineMessage | null {
  if (!isRecord(value)) {
    return null;
  }

  const id = maybeString(value.id);
  const author = maybeString(value.author);
  const body = maybeString(value.body);
  const createdAt =
    typeof value.createdAt === "number" && Number.isFinite(value.createdAt)
      ? value.createdAt
      : null;
  if (!id || !author || !body || createdAt === null) {
    return null;
  }

  const message = { ...value } as TimelineMessage;
  message.id = id;
  message.author = author;
  message.body = body;
  message.createdAt = createdAt;
  message.depth =
    typeof value.depth === "number" && Number.isFinite(value.depth)
      ? value.depth
      : 0;
  message.time = maybeString(value.time) ?? "";
  message.pubkey = maybeString(value.pubkey);
  message.parentId = maybeNullableString(value.parentId);
  message.rootId = maybeNullableString(value.rootId);
  message.avatarUrl = maybeNullableString(value.avatarUrl);
  message.renderKey = maybeString(value.renderKey);

  return message;
}

function parseStoredAgentConversation(
  value: unknown,
): AgentConversation | null {
  if (!isRecord(value)) {
    return null;
  }

  const id = maybeString(value.id);
  const agentName = maybeString(value.agentName);
  const agentPubkey = maybeString(value.agentPubkey);
  const channelId = maybeString(value.channelId);
  const channelName = maybeString(value.channelName);
  const threadRootId = maybeString(value.threadRootId);
  const title = maybeString(value.title);
  const titleStatus =
    value.titleStatus === "provisional" || value.titleStatus === "resolved"
      ? value.titleStatus
      : null;
  const createdAt =
    typeof value.createdAt === "number" && Number.isFinite(value.createdAt)
      ? value.createdAt
      : null;
  const agentReply = parseStoredTimelineMessage(value.agentReply);
  const contextMessages = Array.isArray(value.contextMessages)
    ? value.contextMessages
        .map(parseStoredTimelineMessage)
        .filter((message): message is TimelineMessage => message !== null)
    : [];
  const parentMessage =
    value.parentMessage == null
      ? null
      : parseStoredTimelineMessage(value.parentMessage);
  const threadRootMessage =
    value.threadRootMessage == null
      ? null
      : parseStoredTimelineMessage(value.threadRootMessage);

  if (
    !id ||
    !agentName ||
    !agentPubkey ||
    !agentReply ||
    !channelId ||
    !channelName ||
    createdAt === null ||
    !threadRootId ||
    !title ||
    !titleStatus
  ) {
    return null;
  }

  return {
    id,
    agentName,
    agentPubkey,
    agentReply,
    channelId,
    channelName,
    contextMessages,
    createdAt,
    parentMessage,
    threadRootId,
    threadRootMessage,
    title,
    titleStatus,
  };
}

export function readPersistedAgentConversations(
  pubkey: string,
): AgentConversation[] {
  try {
    const raw = window.localStorage.getItem(
      agentConversationsStorageKey(pubkey),
    );
    if (!raw) {
      return [];
    }

    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }

    const byId = new Map<string, AgentConversation>();
    for (const value of parsed) {
      const conversation = parseStoredAgentConversation(value);
      if (conversation) {
        byId.set(conversation.id, conversation);
      }
    }

    return [...byId.values()]
      .sort((left, right) => right.createdAt - left.createdAt)
      .slice(0, MAX_PERSISTED_AGENT_CONVERSATIONS);
  } catch {
    return [];
  }
}

export function writePersistedAgentConversations(
  pubkey: string,
  conversations: readonly AgentConversation[],
): void {
  try {
    const byId = new Map<string, AgentConversation>();
    for (const conversation of conversations) {
      byId.set(conversation.id, conversation);
    }

    const persisted = [...byId.values()]
      .sort((left, right) => right.createdAt - left.createdAt)
      .slice(0, MAX_PERSISTED_AGENT_CONVERSATIONS);
    window.localStorage.setItem(
      agentConversationsStorageKey(pubkey),
      JSON.stringify(persisted),
    );
  } catch {
    // Best-effort local preference; ignore storage failures.
  }
}

export function buildAgentConversation(
  input: OpenAgentConversationInput,
): AgentConversation {
  const threadRootId =
    input.threadRootMessage?.id ??
    input.agentReply.rootId ??
    input.agentReply.parentId ??
    input.agentReply.id;
  const contextMessages = collectConversationContextMessages(
    input,
    threadRootId,
  );
  const { status: titleStatus, title } = deriveTitleFromContext({
    agentPubkey: input.agentPubkey,
    agentReply: input.agentReply,
    contextMessages,
    parentMessage: input.parentMessage,
    threadRootId,
    threadRootMessage: input.threadRootMessage,
  });

  return {
    id: `${input.channel.id}:${input.agentPubkey}:${input.agentReply.id}`,
    agentName: input.agentName,
    agentPubkey: input.agentPubkey,
    agentReply: input.agentReply,
    channelId: input.channel.id,
    channelName: input.channel.name,
    contextMessages,
    createdAt: Math.max(
      input.agentReply.createdAt,
      input.threadRootMessage?.createdAt ?? 0,
      input.parentMessage?.createdAt ?? 0,
      ...contextMessages.map((message) => message.createdAt),
    ),
    parentMessage: input.parentMessage,
    threadRootId,
    threadRootMessage: input.threadRootMessage,
    title,
    titleStatus,
  };
}

function getTagValue(tags: string[][], name: string): string | null {
  return tags.find((tag) => tag[0] === name)?.[1] ?? null;
}

function getMarkedEventId(tags: string[][], marker: string): string | null {
  return (
    tags.find(
      (tag) =>
        tag[0] === "e" &&
        typeof tag[1] === "string" &&
        tag[1].length > 0 &&
        tag[3] === marker,
    )?.[1] ?? null
  );
}

function parseMarkerContent(content: string): Record<string, unknown> {
  try {
    const parsed = JSON.parse(content);
    return typeof parsed === "object" && parsed !== null
      ? (parsed as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

function trimmedString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

export function parseAgentConversationMarker(
  event: RelayEvent,
): AgentConversationMarker | null {
  if (
    event.kind !== KIND_AGENT_CONVERSATION &&
    event.kind !== KIND_AGENT_CONVERSATION_COMPAT
  ) {
    return null;
  }

  const content = parseMarkerContent(event.content);
  const channelId = getTagValue(event.tags, "h");
  const threadRootId =
    getMarkedEventId(event.tags, "root") ??
    (typeof content.threadRootId === "string" ? content.threadRootId : null);
  const agentReplyId =
    getMarkedEventId(event.tags, "agent-reply") ??
    (typeof content.agentReplyId === "string" ? content.agentReplyId : null);
  const agentPubkey =
    getTagValue(event.tags, "p") ??
    (typeof content.agentPubkey === "string" ? content.agentPubkey : null);
  const parentMessageId =
    typeof content.parentMessageId === "string"
      ? content.parentMessageId
      : null;
  const threadRootMessageId =
    typeof content.threadRootMessageId === "string"
      ? content.threadRootMessageId
      : null;
  const agentName = trimmedString(content.agentName) || agentPubkey || "Agent";
  const title =
    trimmedString(content.title) ??
    getTagValue(event.tags, "title") ??
    "New conversation";
  const titleStatus =
    content.titleStatus === "provisional" ? "provisional" : "resolved";
  const summary = trimmedString(content.summary);
  const summaryCreatedAt =
    typeof content.summaryCreatedAt === "number" &&
    Number.isFinite(content.summaryCreatedAt)
      ? content.summaryCreatedAt
      : null;
  const startedAt =
    typeof content.startedAt === "number" && Number.isFinite(content.startedAt)
      ? content.startedAt
      : event.created_at;

  if (!channelId || !threadRootId || !agentReplyId || !agentPubkey) {
    return null;
  }

  return {
    agentName,
    agentPubkey,
    agentReplyId,
    channelId,
    createdAt: event.created_at,
    eventId: event.id,
    parentMessageId,
    startedAt,
    starterPubkey: event.pubkey,
    summary,
    summaryAuthorName: trimmedString(content.summaryAuthorName),
    summaryAuthorPubkey: trimmedString(content.summaryAuthorPubkey),
    summaryCreatedAt,
    threadRootMessageId,
    threadRootId,
    title,
    titleStatus,
  };
}

export function buildAgentConversationMarkers(
  events: readonly RelayEvent[],
): AgentConversationMarker[] {
  const byAgentReplyId = new Map<string, AgentConversationMarker>();

  for (const event of events) {
    const marker = parseAgentConversationMarker(event);
    if (!marker) {
      continue;
    }

    const current = byAgentReplyId.get(marker.agentReplyId);
    if (
      !current ||
      marker.createdAt > current.createdAt ||
      (marker.createdAt === current.createdAt &&
        marker.eventId > current.eventId)
    ) {
      byAgentReplyId.set(marker.agentReplyId, {
        ...marker,
        startedAt: Math.min(
          current?.startedAt ?? marker.startedAt,
          marker.startedAt,
        ),
        summary: marker.summary ?? current?.summary ?? null,
        summaryAuthorName:
          marker.summary != null
            ? marker.summaryAuthorName
            : (current?.summaryAuthorName ?? null),
        summaryAuthorPubkey:
          marker.summary != null
            ? marker.summaryAuthorPubkey
            : (current?.summaryAuthorPubkey ?? null),
        summaryCreatedAt:
          marker.summary != null
            ? marker.summaryCreatedAt
            : (current?.summaryCreatedAt ?? null),
      });
    } else if (marker.startedAt < current.startedAt) {
      byAgentReplyId.set(marker.agentReplyId, {
        ...current,
        startedAt: marker.startedAt,
        summary: current.summary ?? marker.summary,
        summaryAuthorName: current.summary
          ? current.summaryAuthorName
          : marker.summaryAuthorName,
        summaryAuthorPubkey: current.summary
          ? current.summaryAuthorPubkey
          : marker.summaryAuthorPubkey,
        summaryCreatedAt: current.summary
          ? current.summaryCreatedAt
          : marker.summaryCreatedAt,
      });
    } else if (current.summary == null && marker.summary != null) {
      byAgentReplyId.set(marker.agentReplyId, {
        ...current,
        summary: marker.summary,
        summaryAuthorName: marker.summaryAuthorName,
        summaryAuthorPubkey: marker.summaryAuthorPubkey,
        summaryCreatedAt: marker.summaryCreatedAt,
      });
    }
  }

  return [...byAgentReplyId.values()].sort(
    (left, right) => right.createdAt - left.createdAt,
  );
}

export async function publishAgentConversationMarker(
  input: OpenAgentConversationInput,
  update: AgentConversationMarkerUpdate = {},
): Promise<RelayEvent> {
  const conversation = buildAgentConversation(input);
  const startedAt = Math.floor(Date.now() / 1_000);
  const parentMessageId = input.parentMessage?.id ?? null;
  const threadRootMessageId = input.threadRootMessage?.id ?? null;
  const summary = update.summary?.trim() || null;
  const summaryAuthorName = update.summaryAuthorName?.trim() || null;
  const summaryAuthorPubkey = update.summaryAuthorPubkey?.trim() || null;
  const content = JSON.stringify({
    version: 1,
    title: conversation.title,
    titleStatus: conversation.titleStatus,
    agentName: conversation.agentName,
    agentPubkey: conversation.agentPubkey,
    startedAt,
    threadRootId: conversation.threadRootId,
    threadRootMessageId,
    parentMessageId,
    agentReplyId: conversation.agentReply.id,
    ...(summary
      ? {
          summary,
          summaryAuthorName,
          summaryAuthorPubkey,
          summaryCreatedAt: update.summaryCreatedAt ?? null,
        }
      : {}),
  });
  const event = await signRelayEvent({
    kind: KIND_AGENT_CONVERSATION_COMPAT,
    content,
    tags: [
      ["h", conversation.channelId],
      ["e", conversation.threadRootId, "", "root"],
      ["e", conversation.agentReply.id, "", "agent-reply"],
      ["p", conversation.agentPubkey],
      ["title", conversation.title],
    ],
  });

  return relayClient.publishEvent(
    event,
    "Timed out opening the agent conversation.",
    "Failed to open the agent conversation.",
  );
}

export function getHiddenAgentConversationMessageIds(
  messages: readonly TimelineMessage[],
  markers: readonly AgentConversationMarker[] | undefined,
): Set<string> {
  if (!markers?.length || messages.length === 0) {
    return new Set();
  }

  const orderedMessages = messages
    .map((message, originalIndex) => ({ message, originalIndex }))
    .sort(
      (left, right) =>
        left.message.createdAt - right.message.createdAt ||
        left.originalIndex - right.originalIndex,
    );
  const messageIndexById = new Map(
    orderedMessages.map(({ message }, index) => [message.id, index]),
  );
  const cutoffByThreadRootId = new Map<
    string,
    {
      anchorIndex: number | null;
      anchorMessageId: string;
      startedAt: number;
    }
  >();
  for (const marker of markers) {
    const current = cutoffByThreadRootId.get(marker.threadRootId);
    const candidate = {
      anchorIndex: messageIndexById.get(marker.agentReplyId) ?? null,
      anchorMessageId: marker.agentReplyId,
      startedAt: marker.startedAt,
    };
    const isEarlier =
      current === undefined ||
      (candidate.anchorIndex !== null && current.anchorIndex !== null
        ? candidate.anchorIndex < current.anchorIndex
        : candidate.startedAt < current.startedAt);
    if (isEarlier) {
      cutoffByThreadRootId.set(marker.threadRootId, candidate);
    }
  }

  const hiddenIds = new Set<string>();
  for (const { message } of orderedMessages) {
    const threadRootId = message.rootId ?? message.parentId ?? null;
    if (!threadRootId || message.id === threadRootId) {
      continue;
    }

    const cutoff = cutoffByThreadRootId.get(threadRootId);
    if (cutoff === undefined || message.id === cutoff.anchorMessageId) {
      continue;
    }

    const messageIndex = messageIndexById.get(message.id);
    if (cutoff.anchorIndex !== null && messageIndex !== undefined) {
      if (messageIndex > cutoff.anchorIndex) {
        hiddenIds.add(message.id);
      }
      continue;
    }

    if (message.createdAt > cutoff.startedAt) {
      hiddenIds.add(message.id);
    }
  }

  return hiddenIds;
}
