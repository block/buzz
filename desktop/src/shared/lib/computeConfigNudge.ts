import type { ConfigNudgePayload } from "@/shared/lib/configNudge";
import { extractConfigNudge } from "@/shared/lib/configNudge";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Pure helper that computes the active `ConfigNudgePayload` for a message body.
 *
 * Called by `MarkdownInner` inside a `useMemo`; when the return value is
 * non-null the markdown prose node is suppressed
 * (`configNudge === null ? markdownNode : null`) and replaced by
 * `ConfigNudgeCard`.
 *
 * Extracted into its own module so it can be imported and tested without
 * pulling in `markdown.tsx`'s heavy dependency chain (Tauri, emoji-mart, etc.).
 */
export function computeConfigNudge(
  content: string,
  interactive: boolean,
  configNudgeAuthorPubkey: string | undefined | null,
): ConfigNudgePayload | null {
  if (!interactive || !configNudgeAuthorPubkey) return null;
  const payload = extractConfigNudge(content);
  if (payload === null) return null;
  if (
    normalizePubkey(payload.agent_pubkey) !==
    normalizePubkey(configNudgeAuthorPubkey)
  ) {
    return null;
  }
  return payload;
}
