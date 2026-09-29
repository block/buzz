/**
 * Buzz Term handoff "Return path (Buzz)" — tells the Term agent how to hand
 * back to the **origin** thread (where JM launched Term), not whatever thread
 * was summarized as context.
 *
 * Prefer origin IDs from a `buzz://message` / `hulabuzz://message` deep link
 * when one invoked the card; otherwise use explicit channel/thread ids from
 * the Term Go control.
 */

import { parseMessageLink } from "@/features/messages/lib/messageLink.ts";

export type TermSessionReturnPathOrigin = {
  channelId: string;
  threadId: string;
  /** Present when resolved from a message deep link. */
  messageId?: string | null;
};

export type BuildTermSessionReturnPathSectionInput = {
  originChannelId: string;
  originThreadId: string;
  /**
   * Plain mention string including leading `@` (e.g. `@Fable`).
   * Target is the agent that should pick the thread up next — usually the
   * agent selected in the Term handoff popover — not the Term CLI itself.
   */
  mention: string;
  /**
   * Hex pubkey for the mention agent. Required for real composer chips —
   * plain `@Name` in content alone is not enough.
   */
  mentionPubkey?: string | null;
  /**
   * When the card summarizes a different thread than origin, label both.
   * Omit when summarized === origin (or unknown).
   */
  summarizedChannelId?: string | null;
  summarizedThreadId?: string | null;
};

export type ResolveReturnPathOriginInput = {
  /** `buzz://message?…` / `hulabuzz://message?…` when the card was invoked from a link. */
  deepLink?: string | null;
  /** Fallback when no deep link (Term Go on a thread). */
  channelId?: string | null;
  threadId?: string | null;
  messageId?: string | null;
};

export type ResolveReturnPathMentionInput = {
  /** Explicit override (already may include `@`). */
  explicitMention?: string | null;
  /** Invoking / selected agent display name from the Term handoff roster. */
  agentDisplayName?: string | null;
  /** Channel assistant display name when no agent was selected. */
  channelAssistantName?: string | null;
};

/**
 * Normalize a display name into `@Name`. Does not invent a default agent.
 * Returns empty string when no usable name is available.
 */
export function formatReturnPathMention(
  raw: string | null | undefined,
): string {
  const trimmed = (raw ?? "").trim();
  if (!trimmed) return "";
  return trimmed.startsWith("@") ? trimmed : `@${trimmed}`;
}

/**
 * Pick mention-to-use from the best available signal.
 * Priority: explicitMention → agentDisplayName → channelAssistantName.
 * Never hardcodes a specific agent (e.g. Fable).
 */
export function resolveTermSessionReturnPathMention(
  input: ResolveReturnPathMentionInput,
): string {
  return (
    formatReturnPathMention(input.explicitMention) ||
    formatReturnPathMention(input.agentDisplayName) ||
    formatReturnPathMention(input.channelAssistantName) ||
    ""
  );
}

/**
 * Resolve origin channel/thread for Return path.
 * Prefers parsing a message deep link (`channel`, `thread`, `id`); falls back
 * to explicit ids. When the link has `id` but no `thread`, uses `id` as the
 * thread root (top-level message).
 */
export function resolveTermSessionReturnPathOrigin(
  input: ResolveReturnPathOriginInput,
): TermSessionReturnPathOrigin | null {
  const link = input.deepLink?.trim();
  if (link) {
    const parsed = parseMessageLink(link);
    if (parsed.ok) {
      const { channelId, messageId, threadRootId } = parsed.value;
      const threadId = (threadRootId ?? messageId).trim();
      if (channelId.trim() && threadId) {
        return {
          channelId: channelId.trim(),
          threadId,
          messageId: messageId.trim() || null,
        };
      }
    }
  }

  const channelId = input.channelId?.trim() ?? "";
  const threadId = input.threadId?.trim() ?? "";
  if (channelId && threadId) {
    return {
      channelId,
      threadId,
      messageId: input.messageId?.trim() || null,
    };
  }
  return null;
}

function sameThread(
  aChannel: string,
  aThread: string,
  bChannel: string | null | undefined,
  bThread: string | null | undefined,
): boolean {
  const bc = (bChannel ?? "").trim();
  const bt = (bThread ?? "").trim();
  if (!bc || !bt) return true;
  return aChannel === bc && aThread === bt;
}

/**
 * Build the `## Return path (Buzz)` markdown block for the term-session
 * card `prompt`. IDs must already be the **origin** (launch) thread.
 */
export function buildTermSessionReturnPathSection(
  input: BuildTermSessionReturnPathSectionInput,
): string {
  const originChannelId = input.originChannelId.trim();
  const originThreadId = input.originThreadId.trim();
  const mention = formatReturnPathMention(input.mention) || "@agent";
  const displayName = mention.startsWith("@") ? mention.slice(1) : mention;
  const mentionPubkey = (input.mentionPubkey ?? "").trim();

  const lines = [
    "## Return path (Buzz)",
    `- origin channelId: ${originChannelId}`,
    `- origin threadId:  ${originThreadId}`,
    `- mention to use:   ${mention}`,
  ];

  if (mentionPubkey) {
    lines.push(`- mention pubkey:   ${mentionPubkey}`);
  }

  if (
    !sameThread(
      originChannelId,
      originThreadId,
      input.summarizedChannelId,
      input.summarizedThreadId,
    )
  ) {
    lines.push(
      `- summarized/source channelId: ${(input.summarizedChannelId ?? "").trim()}`,
      `- summarized/source threadId:  ${(input.summarizedThreadId ?? "").trim()}`,
    );
  }

  const mentionsJson = mentionPubkey
    ? `[{ displayName: "${displayName}", pubkey: "${mentionPubkey}", isAgent: true }]`
    : `[{ displayName: "${displayName}", pubkey: "<agent-pubkey>", isAgent: true }]`;

  lines.push(
    '- On "report back" / "hand back" / "I\'m done":',
    "  call buzz_draft_message with the origin channelId + threadId,",
    `  content starting with "${mention} <text JM asked for>",`,
    `  AND mentions: ${mentionsJson}`,
    "  for that agent. Plain @Name alone is NOT enough — drafts need mentionRefs.",
    "  Draft only. JM clicks Send.",
    "- Never draft to any other channel or thread unless JM gives new IDs.",
    "- Never auto-draft progress; keep status in the Term TUI.",
  );

  return lines.join("\n");
}
