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
    const attachedMessageId =
      promptMessageId && messageIds.has(promptMessageId)
        ? promptMessageId
        : null;
    const renderBlock = {
      id: getBlockId(block),
      block,
      attachedMessageId,
      suppressPromptMessage: attachedMessageId !== null,
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

    const keepMessage =
      [...agentRun]
        .reverse()
        .find((message) => isHumanFacingAssistantText(message.content)) ??
      agentRun[agentRun.length - 1];

    for (const message of agentRun) {
      if (message.id !== keepMessage.id) {
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

  for (const segment of block.segments) {
    const messageId = getSegmentPromptMessageId(segment);
    if (messageId) {
      return messageId;
    }
  }
  return null;
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
