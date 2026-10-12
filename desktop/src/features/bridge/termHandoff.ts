// Desktop-side executor for the bridge command `term.handoff` (web "Pass thread context to Buzz Term").
// Runs the same flow as the desktop card (TermSessionHandoffPopover → agent builds a term-session card → card Open):
// 1. post the handoff instruction to the chosen agent in the thread (buildTermSessionHandoffInstruction),
// 2. wait for that agent's `term-session` card reply,
// 3. open it in the desktop's Buzz Term (openTermSessionCard code path) and report what the web needs to attach.
// Pure: all IO is injected so it is unit-testable.
import { buildTermSessionHandoffInstruction } from "../term-session/lib/handoffInstruction.ts";
import type { TermSessionCard, TermSessionTool } from "../term-session/lib/types.ts";
import { shellSingleQuote } from "../term-session/lib/launchCommand.ts";
import { TERM_HANDOFF_WAIT_MS } from "./protocol.ts";

export class TermHandoffError extends Error {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

export type TermHandoffInput = {
  agentPubkey: string;
  channelId: string;
  threadId: string;
  harness: TermSessionTool;
};

export type TermHandoffDeps = {
  agentName(pubkey: string): string;
  channelName(channelId: string): string;
  harnessAvailable(h: TermSessionTool): boolean;
  /** Posts the instruction as a thread reply that @-mentions the agent. */
  sendInstruction(input: TermHandoffInput, content: string): Promise<void>;
  /** Resolves with the first term-session card the agent posts in that thread after `sinceMs`, or null on timeout. */
  waitForCard(input: TermHandoffInput, sinceMs: number, timeoutMs: number): Promise<TermSessionCard | null>;
  /** Desktop `openTermSessionCard` path: prepares the prompt, opens the desktop Buzz Term tab, returns the launch line. */
  openCard(
    card: TermSessionCard,
    ctx: { channelId: string; channelName: string; threadId: string },
  ): Promise<{ launchCommand: string; extraEnv: Record<string, string> } | null>;
  now?(): number;
  waitMs?: number;
};

export type TermHandoffResult = {
  sid: string;
  name: string;
  tool: TermSessionTool;
  /** Shell line that starts the session (reads the prompt file on this desktop). Contains no prompt text or secrets. */
  launchCommand: string;
  channelId: string;
  threadId: string;
};

export async function runTermHandoff(deps: TermHandoffDeps, input: TermHandoffInput): Promise<TermHandoffResult> {
  if (!deps.harnessAvailable(input.harness))
    throw new TermHandoffError("harness_unavailable", `${input.harness === "claude" ? "Claude" : "Codex"} isn't installed on your desktop.`);
  const name = deps.agentName(input.agentPubkey) || input.agentPubkey.slice(0, 12);
  const content = buildTermSessionHandoffInstruction({
    agentDisplayName: name,
    channelId: input.channelId,
    threadId: input.threadId,
    harness: input.harness,
    mentionPubkey: input.agentPubkey,
  });
  const since = (deps.now ?? Date.now)();
  // Subscribe before sending so a fast reply is never missed.
  const waiting = deps.waitForCard(input, since, deps.waitMs ?? TERM_HANDOFF_WAIT_MS);
  waiting.catch(() => {});
  try {
    await deps.sendInstruction(input, content);
  } catch (e) {
    throw new TermHandoffError("send_failed", `Could not ask the agent: ${String(e instanceof Error ? e.message : e).slice(0, 120)}`);
  }
  const card = await waiting;
  if (!card) throw new TermHandoffError("no_card", `${name} didn't produce a handoff card in time. Check the thread on your desktop.`);
  const launched = await deps.openCard(card, {
    channelId: input.channelId,
    channelName: deps.channelName(input.channelId),
    threadId: input.threadId,
  });
  if (!launched) throw new TermHandoffError("open_failed", "The desktop could not start the Buzz Term session.");
  return {
    sid: card.sid,
    name: card.name.slice(0, 120),
    tool: card.tool,
    launchCommand: withEnvExports(launched.launchCommand, launched.extraEnv),
    channelId: input.channelId,
    threadId: input.threadId,
  };
}

/** The web attaches through a fresh PTY (no spawn-time env), so the session env is exported in the typed line. Paths only, no secrets. */
export function withEnvExports(launchCommand: string, extraEnv: Record<string, string>): string {
  const exports = Object.entries(extraEnv)
    .filter(([k, v]) => /^[A-Z_][A-Z0-9_]*$/.test(k) && !launchCommand.includes(`export ${k}=`) && typeof v === "string" && v.length > 0)
    .map(([k, v]) => `export ${k}=${shellSingleQuote(v)}`);
  return [...exports, launchCommand].join(" && ");
}

/** True for a card event that answers this handoff: from the agent, in this thread, after we asked. */
export function isHandoffReply(
  ev: { pubkey: string; created_at: number; tags: string[][] },
  input: Pick<TermHandoffInput, "agentPubkey" | "threadId">,
  sinceS: number,
): boolean {
  return (
    ev.pubkey.toLowerCase() === input.agentPubkey.toLowerCase() &&
    ev.created_at >= sinceS - 1 &&
    ev.tags.some((t) => t[0] === "e" && t[1] === input.threadId)
  );
}
