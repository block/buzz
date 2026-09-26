import type { TermSessionTool } from "./types.ts";

export type BuildTermSessionHandoffInstructionInput = {
  agentDisplayName: string;
  channelId: string;
  threadId: string;
  harness: TermSessionTool;
  openclawWorkspace?: boolean;
};

/**
 * Thread reply body that @-mentions the selected agent and asks it to call
 * buzz-dev-mcp `term_session_card`, then paste only the tool fence + a short ack.
 */
export function buildTermSessionHandoffInstruction(
  input: BuildTermSessionHandoffInstructionInput,
): string {
  const name = input.agentDisplayName.trim() || "agent";
  const openclawLine = input.openclawWorkspace
    ? '- Pass openclawWorkspace: true (boolean only; never put tokens/JWTs in the card).'
    : "- Omit openclawWorkspace (or set false).";

  return [
    `@${name} Summarize this thread for a Buzz Term handoff.`,
    "",
    "Call buzz-dev-mcp tool `term_session_card` with:",
    `- tool / harness: ${input.harness}`,
    "- name: short title",
    "- prompt: full handoff text (ONLY in this JSON field; UI hides it).",
    "  In that prompt, tell Term: progress stays in the TUI; do not @mention the",
    "  user or call buzz_draft_message except for content the human asked to send.",
    "- optional cwd, summary, sid",
    openclawLine,
    "",
    "Context for your summary:",
    `- channelId: ${input.channelId}`,
    `- threadId: ${input.threadId}`,
    `- harness: ${input.harness}`,
    "",
    "Reply in chat with ONLY a short one-line ack plus the tool’s returned",
    "```term-session``` fence. Do not dump the prompt as plain markdown.",
    "Do not invent the fence by hand — paste the tool output exactly.",
  ].join("\n");
}
