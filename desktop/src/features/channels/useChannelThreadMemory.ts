import * as React from "react";

import { rememberChannelThread } from "@/features/channels/channelPanelMemory";

/**
 * Continuously mirrors the channel's `?thread` search value into the
 * per-channel panel memory (`channelPanelMemory.ts`), so `goChannel` can
 * restore the thread panel when the user returns to the channel.
 *
 * Recording continuously (rather than on exit) keeps every leave-path correct
 * for free: closing the panel records `null`, leaving via Home/Settings needs
 * no hook, and a stale thread snap-closed by `useThreadTargetSync` records
 * `null` so it is forgotten rather than retried.
 *
 * Records the raw URL value, not `effectiveOpenThreadHeadId` — the effective
 * id is huddle-suppressed/optimistic and does not represent what a returning
 * visit should restore. Huddle transcripts force-close threads
 * (`useHuddleThreadIsolation`) and the forum-post view does not use `thread`,
 * so both are skipped.
 */
export function useChannelThreadMemory({
  activeChannelId,
  isHuddleTranscript,
  openThreadHeadId,
  selectedForumPostId,
}: {
  activeChannelId: string | null;
  isHuddleTranscript: boolean;
  openThreadHeadId: string | null;
  selectedForumPostId: string | null;
}) {
  React.useEffect(() => {
    if (!activeChannelId || isHuddleTranscript || selectedForumPostId) {
      return;
    }
    rememberChannelThread(activeChannelId, openThreadHeadId);
  }, [
    activeChannelId,
    isHuddleTranscript,
    openThreadHeadId,
    selectedForumPostId,
  ]);
}
