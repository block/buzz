import { sanitizeChannelName } from "@/features/dev-mode/lib/sessionNaming";
import { meshNodeStatus } from "@/shared/api/tauriMesh";

/**
 * Ask an LLM to title a session channel from its first prompt.
 *
 * The desktop app has no general one-shot completion API; the only local
 * inference surface is the mesh LLM node's OpenAI-compatible endpoint, used
 * when a node is running with a served model. Returns null (caller keeps the
 * slug-derived name) when no LLM is reachable — callers must not block
 * channel creation on this.
 */

const TITLE_TIMEOUT_MS = 8_000;

export async function generateChannelTitle(
  prompt: string,
): Promise<string | null> {
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
          {
            role: "system",
            content:
              "You name chat channels. Reply with only a 2-5 word kebab-case " +
              "channel name summarizing the user's prompt. No punctuation " +
              "besides hyphens, no explanation.",
          },
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
    const name = sanitizeChannelName(content.replaceAll("-", " "));
    return name.length >= 3 ? name : null;
  } catch {
    return null;
  }
}
