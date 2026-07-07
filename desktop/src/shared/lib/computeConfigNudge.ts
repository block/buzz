import type { ReactNode } from "react";
import type { ConfigNudgePayload } from "@/shared/lib/configNudge";
import { extractConfigNudge } from "@/shared/lib/configNudge";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Pure helper that computes the active `ConfigNudgePayload` for a message body.
 *
 * Called by `MarkdownInner` inside a `useMemo`; when the return value is
 * non-null the markdown prose node is suppressed (via `selectProseOrNudge`)
 * and replaced by `ConfigNudgeCard`.
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

/**
 * Returns `markdownNode` when no trusted config-nudge payload is present,
 * or `null` when the card should render instead.
 *
 * This is the single production copy of the prose-suppression branch.
 * `MarkdownInner` calls this instead of an inline ternary so the test suite
 * can import and exercise the exact same branch, making a suppression revert
 * observable at unit-test time.
 */
export function selectProseOrNudge(
  configNudge: ConfigNudgePayload | null,
  markdownNode: ReactNode,
): ReactNode {
  return configNudge === null ? markdownNode : null;
}
