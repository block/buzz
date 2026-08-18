/**
 * URL search-param keys managed by the channel-panel history state.
 *
 * Kept in a separate pure module (no React / router imports) so tests can
 * import and assert the patch contracts without a browser environment.
 */

export const CHANNEL_SEARCH_KEYS = [
  "agentSession",
  "agentSessionChannel",
  "autoSend",
  "channelManagement",
  "messageId",
  "profile",
  "profileTab",
  "profileView",
  "thread",
  "threadRootId",
] as const;

export type ChannelSearchKey = (typeof CHANNEL_SEARCH_KEYS)[number];

/**
 * Returns the search-param patch that clears only the `autoSend` trigger.
 *
 * Exported so tests can verify the patch is surgical — it must not include
 * `thread` or any other panel key that would collapse open panels. This is
 * the regression guard for the "auto-submit clear drops the thread route"
 * defect: a `goChannel()` re-navigation drops every search key including
 * `thread`; this patch removes only `autoSend` via `applyPatch`, preserving
 * the thread panel across the deferred `setTimeout(0)` submit.
 */
export function buildAutoSendClearPatch(): Partial<
  Record<ChannelSearchKey, string | null>
> {
  return { autoSend: null };
}
