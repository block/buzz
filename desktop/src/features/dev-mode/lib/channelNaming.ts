import { generateAgentCompletion } from "@/shared/api/agentCompletion";
import { sanitizeChannelName } from "@/features/dev-mode/lib/sessionNaming";
import { meshNodeStatus } from "@/shared/api/tauriMesh";

/**
 * Ask an LLM to title a session channel from its first prompt.
 *
 * Prefers a one-shot completion through a managed agent (the agent tagged in
 * the composer, or any configured managed agent) via `buzz-acp complete` —
 * a private subprocess exchange that never appears in a channel. Falls back
 * to the mesh LLM node's OpenAI-compatible endpoint when one is running.
 * Returns null (caller keeps its placeholder name) when neither produces a
 * usable title — callers must not block channel creation on this.
 */

const TITLE_TIMEOUT_MS = 8_000;

const TITLE_INSTRUCTION =
  "Reply with only a 2-5 word kebab-case channel name summarizing the " +
  "following chat prompt. No punctuation besides hyphens, no quotes, no " +
  "explanation — just the name.";

/** Reduce raw LLM output to a valid channel name, or null if unusable. */
function toChannelName(raw: string): string | null {
  // Keep only the last non-empty line — chatty agents sometimes preface the
  // answer even when told not to.
  const lastLine = raw
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .at(-1);
  if (!lastLine) return null;
  const name = sanitizeChannelName(lastLine.replaceAll("-", " "));
  return name.length >= 3 ? name : null;
}

async function agentTitle(
  prompt: string,
  agentPubkey: string,
): Promise<string | null> {
  try {
    const text = await generateAgentCompletion({
      pubkey: agentPubkey,
      prompt: `${TITLE_INSTRUCTION}\n\nPrompt:\n${prompt.slice(0, 500)}`,
      systemPrompt: TITLE_INSTRUCTION,
    });
    return toChannelName(text);
  } catch {
    return null;
  }
}

async function meshTitle(prompt: string): Promise<string | null> {
  let apiBaseUrl: string;
  let modelId: string;
  try {
    const status = await meshNodeStatus();
    if (status.state !== "running" || !status.apiBaseUrl || !status.modelId) {
      return null;
    }
    apiBaseUrl = status.apiBaseUrl;
    modelId = status.modelId;
  } catch {
    return null;
  }

  try {
    const response = await fetch(`${apiBaseUrl}/chat/completions`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        model: modelId,
        messages: [
          { role: "system", content: TITLE_INSTRUCTION },
          { role: "user", content: prompt.slice(0, 500) },
        ],
        max_tokens: 24,
        temperature: 0.2,
      }),
      signal: AbortSignal.timeout(TITLE_TIMEOUT_MS),
    });
    if (!response.ok) return null;
    const payload: unknown = await response.json();
    const content = (
      payload as { choices?: { message?: { content?: unknown } }[] }
    ).choices?.[0]?.message?.content;
    if (typeof content !== "string") return null;
    return toChannelName(content);
  } catch {
    return null;
  }
}

export async function generateChannelTitle(
  prompt: string,
  agentPubkey?: string | null,
): Promise<string | null> {
  if (agentPubkey) {
    const title = await agentTitle(prompt, agentPubkey);
    if (title) return title;
  }
  return meshTitle(prompt);
}
