import type { TermSessionTool } from "./types.ts";
import {
  buildTermSessionReturnPathSection,
  resolveTermSessionReturnPathMention,
  resolveTermSessionReturnPathOrigin,
} from "./returnPath.ts";

export type BuildTermSessionHandoffInstructionInput = {
  agentDisplayName: string;
  channelId: string;
  threadId: string;
  harness: TermSessionTool;
  openclawWorkspace?: boolean;
  /**
   * Optional `buzz://message` / `hulabuzz://message` deep link the handoff was
   * invoked from. When present, origin channel/thread are taken from the link
   * (preferring `thread=`, else message `id=`).
   */
  originDeepLink?: string | null;
  /** Explicit mention override (with or without `@`). Defaults to agentDisplayName. */
  mentionToUse?: string | null;
  /**
   * When the card will summarize a different thread than origin, pass both so
   * the instruction labels origin vs summarized/source.
   */
  summarizedChannelId?: string | null;
  summarizedThreadId?: string | null;
};

/**
 * Thread reply body that @-mentions the selected agent and asks it to call
 * buzz-dev-mcp `term_session_card`, then paste only the tool fence + a short ack.
 *
 * Embeds a filled **Return path (Buzz)** block (origin thread + mention) that
 * the card-maker must put into JSON `prompt` so Term knows how to hand back.
 */
export function buildTermSessionHandoffInstruction(
  input: BuildTermSessionHandoffInstructionInput,
): string {
  const name = input.agentDisplayName.trim() || "agent";
  const openclawLine = input.openclawWorkspace
    ? "- Pass openclawWorkspace: true (boolean only; never put tokens/JWTs in the card)."
    : "- Omit openclawWorkspace (or set false).";

  const origin =
    resolveTermSessionReturnPathOrigin({
      deepLink: input.originDeepLink,
      channelId: input.channelId,
      threadId: input.threadId,
    }) ?? {
      channelId: input.channelId.trim(),
      threadId: input.threadId.trim(),
      messageId: null,
    };

  const mention = resolveTermSessionReturnPathMention({
    explicitMention: input.mentionToUse,
    agentDisplayName: name,
  });

  const returnPath = buildTermSessionReturnPathSection({
    originChannelId: origin.channelId,
    originThreadId: origin.threadId,
    mention,
    summarizedChannelId: input.summarizedChannelId,
    summarizedThreadId: input.summarizedThreadId,
  });

  return [
    `@${name} Summarize this thread for a Buzz Term handoff.`,
    "",
    "Call buzz-dev-mcp tool `term_session_card` with:",
    `- tool / harness: ${input.harness}`,
    "- name: short title",
    "- prompt: full handoff text (ONLY in this JSON field; UI hides it).",
    "  In that prompt, tell Term: progress stays in the TUI; do not @mention the",
    "  user or call buzz_draft_message except for content the human asked to send.",
    "  **Append this exact Return path block at the end of `prompt`** (origin =",
    "  the thread JM launched from — NOT a different thread you may summarize):",
    "",
    returnPath,
    "",
    "- Also pass MCP fields when supported:",
    `-   originChannelId: ${origin.channelId}`,
    `-   originThreadId: ${origin.threadId}`,
    `-   mentionToUse: ${mention}`,
    "- optional cwd, summary, sid",
    openclawLine,
    "",
    "Origin (Return path) — use these IDs for hand-back, always:",
    `- origin channelId: ${origin.channelId}`,
    `- origin threadId: ${origin.threadId}`,
    `- mention to use: ${mention}`,
    "",
    "Context for your summary (may match origin; if you summarize a different",
    "thread, label it summarized/source in the prompt and keep Return path on origin):",
    `- channelId: ${input.channelId}`,
    `- threadId: ${input.threadId}`,
    `- harness: ${input.harness}`,
    "",
    "Reply in chat with ONLY a short one-line ack plus the tool’s returned",
    "```term-session``` fence. Do not dump the prompt as plain markdown.",
    "Do not invent the fence by hand — paste the tool output exactly.",
  ].join("\n");
}
