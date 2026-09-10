import * as React from "react";

import { useThreadWorkingAgentPubkeys } from "@/features/agents/activeAgentTurnsStore";

type BotTypingEntry = { pubkey: string; threadHeadId?: string | null };

/**
 * Working agents for the THREAD composer bar: observer-derived turns scoped to
 * the open thread (frame `sessionId` is the thread root shortened, so prefix
 * match is the identity test), folded with thread-scoped typing entries — the
 * same observer-primary/typing-fallback rule the channel composer bar uses.
 */
export function useThreadComposerWorkingPubkeys(
  channelId: string | null,
  openThreadHeadId: string | null,
  botTypingEntries: readonly BotTypingEntry[],
): string[] {
  const observerPubkeys = useThreadWorkingAgentPubkeys(
    channelId,
    openThreadHeadId,
  );

  return React.useMemo(() => {
    const merged = [...observerPubkeys];
    if (openThreadHeadId) {
      for (const entry of botTypingEntries) {
        if (entry.threadHeadId !== openThreadHeadId) continue;
        if (
          !merged.some(
            (candidate) =>
              candidate.toLowerCase() === entry.pubkey.toLowerCase(),
          )
        ) {
          merged.push(entry.pubkey);
        }
      }
    }
    return merged;
  }, [botTypingEntries, observerPubkeys, openThreadHeadId]);
}
