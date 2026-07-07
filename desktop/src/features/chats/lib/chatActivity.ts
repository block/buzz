import type {
  TranscriptDisplayBlock,
  TranscriptTurnSegment,
} from "@/features/agents/ui/agentSessionTranscriptGrouping";
import { buildTranscriptDisplayBlocks } from "@/features/agents/ui/agentSessionTranscriptGrouping";
import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";
import {
  isHumanFacingAssistantText,
  normalizeAssistantMessageTextForMatching,
} from "@/features/chats/ui/chatActivityText";
import type { RelayEvent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type ChatActivityRenderBlock = {
  id: string;
  block: TranscriptDisplayBlock;
  attachedMessageId: string | null;
  suppressPromptMessage: boolean;
};

export type ChatActivityPlacement = {
  blocksByMessageId: Map<string, ChatActivityRenderBlock[]>;
  unplacedBlocks: ChatActivityRenderBlock[];
  hiddenAgentMessageIds: Set<string>;
  totalBlockCount: number;
};

export function buildChatActivityPlacement({
  agentPubkey,
  messages,
  transcript,
}: {
  agentPubkey?: string | null;
  messages: RelayEvent[];
  transcript: TranscriptItem[];
}): ChatActivityPlacement {
  const messageIds = new Set(messages.map((message) => message.id));
  const blocks = buildTranscriptDisplayBlocks(transcript);
  const blocksByMessageId = new Map<string, ChatActivityRenderBlock[]>();
  const unplacedBlocks: ChatActivityRenderBlock[] = [];

  for (const block of blocks) {
    const promptMessageId = getPromptMessageId(block);
    const exactMessageId =
      promptMessageId && messageIds.has(promptMessageId)
        ? promptMessageId
        : null;
    // A block whose prompt message can't be matched (replayed backlog id
    // outside the fetch window, prompt text that didn't carry the id, …)
    // must NOT drop to the trailing bucket — that pins it below every
    // message that arrives later, forever. Place it at its chronological
    // spot instead: after the latest message that predates the block.
    const attachedMessageId =
      exactMessageId ?? findMessageIdByTime(messages, getBlockStartMs(block));
    const renderBlock = {
      id: getBlockId(block),
      block,
      attachedMessageId,
      suppressPromptMessage: exactMessageId !== null,
    };

    if (attachedMessageId) {
      const existing = blocksByMessageId.get(attachedMessageId) ?? [];
      existing.push(renderBlock);
      blocksByMessageId.set(attachedMessageId, existing);
    } else {
      unplacedBlocks.push(renderBlock);
    }
  }

  const representedAgentTextCounts = collectRepresentedAgentTextCounts(
    transcript,
    agentPubkey,
  );
  const hiddenAgentMessageIds = buildHiddenAgentMessageIds({
    agentPubkey,
    messages,
    representedAgentTextCounts,
  });
  addIntermediateAgentTurnMessageIds({
    agentPubkey,
    hiddenIds: hiddenAgentMessageIds,
    messages,
  });

  return {
    blocksByMessageId,
    hiddenAgentMessageIds,
    unplacedBlocks,
    totalBlockCount: blocks.length,
  };
}

export function shouldHidePersistedAgentMessage({
  event,
  hiddenAgentMessageIds,
}: {
  event: RelayEvent;
  hiddenAgentMessageIds: ReadonlySet<string>;
}) {
  return hiddenAgentMessageIds.has(event.id);
}

function buildHiddenAgentMessageIds({
  agentPubkey,
  messages,
  representedAgentTextCounts,
}: {
  agentPubkey?: string | null;
  messages: RelayEvent[];
  representedAgentTextCounts: Map<string, number>;
}) {
  const hiddenIds = new Set<string>();
  if (!agentPubkey || representedAgentTextCounts.size === 0) {
    return hiddenIds;
  }

  for (const message of [...messages].reverse()) {
    if (normalizePubkey(message.pubkey) !== normalizePubkey(agentPubkey)) {
      continue;
    }
    const text = normalizeMessageText(message.content);
    const remaining = representedAgentTextCounts.get(text) ?? 0;
    if (text.length === 0 || remaining <= 0) {
      continue;
    }
    hiddenIds.add(message.id);
    if (remaining === 1) {
      representedAgentTextCounts.delete(text);
    } else {
      representedAgentTextCounts.set(text, remaining - 1);
    }
  }

  return hiddenIds;
}

function addIntermediateAgentTurnMessageIds({
  agentPubkey,
  hiddenIds,
  messages,
}: {
  agentPubkey?: string | null;
  hiddenIds: Set<string>;
  messages: RelayEvent[];
}) {
  if (!agentPubkey) {
    return;
  }
  const normalizedAgent = normalizePubkey(agentPubkey);
  let agentRun: RelayEvent[] = [];
  let agentRunHasAnchor = false;
  let hasSeenNonAgentMessage = false;

  const flushRun = () => {
    if (!agentRunHasAnchor || agentRun.length <= 1) {
      agentRun = [];
      agentRunHasAnchor = false;
      return;
    }

    // Hide only interim narration: substantive messages (PR announcements,
    // answers) all stay, and the run's FINAL message always stays even when
    // narration-styled — a real reply must never lose to a phrasing
    // heuristic, which is exactly how turns ended up visibly working but
    // never answering.
    const lastId = agentRun[agentRun.length - 1].id;
    for (const message of agentRun) {
      if (
        message.id !== lastId &&
        !isHumanFacingAssistantText(message.content)
      ) {
        hiddenIds.add(message.id);
      }
    }
    agentRun = [];
    agentRunHasAnchor = false;
  };

  for (const message of messages) {
    if (normalizePubkey(message.pubkey) === normalizedAgent) {
      if (normalizeMessageText(message.content).length > 0) {
        if (agentRun.length === 0) {
          agentRunHasAnchor = hasSeenNonAgentMessage;
        }
        agentRun.push(message);
      }
      continue;
    }
    flushRun();
    hasSeenNonAgentMessage = true;
  }
  flushRun();
}

function collectRepresentedAgentTextCounts(
  transcript: TranscriptItem[],
  agentPubkey?: string | null,
) {
  const texts = new Map<string, number>();
  for (const item of transcript) {
    if (item.type !== "message" || item.role !== "assistant") {
      continue;
    }
    if (
      item.authorPubkey &&
      agentPubkey &&
      normalizePubkey(item.authorPubkey) !== normalizePubkey(agentPubkey)
    ) {
      continue;
    }
    const text = normalizeMessageText(item.text);
    if (text.length > 0) {
      texts.set(text, (texts.get(text) ?? 0) + 1);
    }
  }
  return texts;
}

function normalizeMessageText(text: string) {
  return normalizeAssistantMessageTextForMatching(text);
}

// Transcript timestamps come from the agent host's clock while message
// created_at comes from the sender's; a little leeway keeps a block from
// slipping one message early when the agent clock runs slightly behind.
const BLOCK_TIME_SKEW_MS = 5_000;

/**
 * Latest message at or before the block's start (with skew tolerance), for
 * blocks whose prompt message couldn't be matched by id. A block older than
 * every message attaches to the earliest one; null only when there are no
 * messages or the block has no usable timestamp.
 */
function findMessageIdByTime(
  messages: RelayEvent[],
  blockStartMs: number | null,
): string | null {
  if (blockStartMs === null || messages.length === 0) {
    return null;
  }
  let bestId: string | null = null;
  let bestMs = Number.NEGATIVE_INFINITY;
  let earliestId: string | null = null;
  let earliestMs = Number.POSITIVE_INFINITY;
  for (const message of messages) {
    const ms = message.created_at * 1_000;
    if (ms < earliestMs) {
      earliestMs = ms;
      earliestId = message.id;
    }
    if (ms <= blockStartMs + BLOCK_TIME_SKEW_MS && ms >= bestMs) {
      bestMs = ms;
      bestId = message.id;
    }
  }
  return bestId ?? earliestId;
}

function getBlockStartMs(block: TranscriptDisplayBlock): number | null {
  if (block.kind === "single") {
    return parseTimestampMs(block.item.timestamp);
  }
  for (const segment of block.segments) {
    let ms: number | null = null;
    if (segment.kind === "prompt") {
      ms = parseTimestampMs(segment.user.timestamp);
    } else if (segment.kind === "item") {
      ms = parseTimestampMs(segment.item.timestamp);
    } else if (segment.kind === "summary") {
      ms =
        parseTimestampMs(segment.summary.items[0]?.timestamp) ??
        parseTimestampMs(segment.summary.timestamp);
    } else if (segment.kind === "setup") {
      ms = parseTimestampMs(segment.items[0]?.timestamp);
    }
    if (ms !== null) {
      return ms;
    }
  }
  return null;
}

function parseTimestampMs(timestamp: string | undefined): number | null {
  if (!timestamp) {
    return null;
  }
  const ms = Date.parse(timestamp);
  return Number.isFinite(ms) ? ms : null;
}

function getBlockId(block: TranscriptDisplayBlock) {
  if (block.kind === "single") {
    return block.item.id;
  }
  return `turn:${block.turnId}`;
}

function getPromptMessageId(block: TranscriptDisplayBlock) {
  if (block.kind === "single") {
    return getItemPromptMessageId(block.item);
  }

  // A turn can span several of the user's messages (a mid-turn steer, or a
  // batch that merged a replayed backlog message with a fresh one). Attach
  // the activity to the LATEST of them — anchoring to the first would render
  // the turn's output above the user's newest message, pinning that message
  // to the bottom of the conversation.
  let latestMessageId: string | null = null;
  for (const segment of block.segments) {
    const messageId = getSegmentPromptMessageId(segment);
    if (messageId) {
      latestMessageId = messageId;
    }
  }
  return latestMessageId;
}

function getSegmentPromptMessageId(segment: TranscriptTurnSegment) {
  if (segment.kind === "prompt") {
    return segment.user.messageId ?? null;
  }
  if (segment.kind === "item") {
    return getItemPromptMessageId(segment.item);
  }
  if (segment.kind === "summary") {
    for (const item of segment.summary.items) {
      const messageId = getItemPromptMessageId(item);
      if (messageId) return messageId;
    }
  }
  return null;
}

function getItemPromptMessageId(item: TranscriptItem) {
  if (item.type !== "message" || item.role !== "user") {
    return null;
  }
  return item.messageId ?? null;
}
