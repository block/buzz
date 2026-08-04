import { invokeTauri } from "@/shared/api/tauri";

export type GenerateAgentCompletionInput = {
  /** Managed agent pubkey whose configured harness runs the prompt. */
  pubkey: string;
  prompt: string;
  systemPrompt?: string;
};

/**
 * Run one bounded prompt through a managed agent and return its reply text.
 *
 * Spawns a short-lived agent subprocess — no relay traffic, nothing appears
 * in any channel. Slow (agent startup + one turn); callers must not block
 * interactive flows on it.
 */
export async function generateAgentCompletion(
  input: GenerateAgentCompletionInput,
): Promise<string> {
  return invokeTauri<string>("generate_agent_completion", {
    pubkey: input.pubkey,
    prompt: input.prompt,
    systemPrompt: input.systemPrompt ?? null,
  });
}
