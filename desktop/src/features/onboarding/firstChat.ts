import type { OnboardingAgent } from "./agents";

/**
 * First-chat (step 5) message model + reply seam.
 *
 * SCRIPTED FOR NOW: the onboarding agents are placeholder data with no running
 * harness, so we can't do a real ACP round-trip yet. `requestAgentReply` is the
 * single seam to swap in the real path later — replace its body with a real DM
 * send + reply subscription once agents are real and the harness step precedes
 * this one. The UI depends only on this function's Promise contract.
 */

export type ChatRole = "user" | "agent";

export type ChatMessage = {
  id: string;
  role: ChatRole;
  text: string;
  at: number;
};

/** The forced first prompt (per product decision: try to force a haiku). */
export const FIRST_CHAT_PROMPT = "Write me a haiku about getting started.";

/** Simulated agent "thinking" delay before the scripted reply resolves (ms). */
export const SCRIPTED_REPLY_DELAY_MS = 1400;

/**
 * A friendly getting-started haiku. Kept deterministic so the onboarding
 * demo is stable; the real agent will generate its own once wired up.
 */
export function scriptedHaiku(): string {
  return [
    "A fresh page, humming—",
    "small wings lift the first idea,",
    "the swarm finds its way.",
  ].join("\n");
}

/**
 * Request the agent's reply to the user's first message.
 *
 * @returns the agent's reply text.
 *
 * TODO(real-round-trip): send `prompt` as a DM to `agent`'s pubkey via the
 * chosen harness and await the first reply event instead of resolving a
 * scripted haiku. Reject on timeout so the caller can show the retry/skip
 * fallback.
 */
export function requestAgentReply(
  _agent: OnboardingAgent,
  _prompt: string,
): {
  promise: Promise<string>;
  cancel: () => void;
} {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let cancelled = false;

  const promise = new Promise<string>((resolve, reject) => {
    timer = setTimeout(() => {
      if (cancelled) {
        reject(new Error("cancelled"));
        return;
      }
      resolve(scriptedHaiku());
    }, SCRIPTED_REPLY_DELAY_MS);
  });

  return {
    promise,
    cancel: () => {
      cancelled = true;
      if (timer) {
        clearTimeout(timer);
      }
    },
  };
}
