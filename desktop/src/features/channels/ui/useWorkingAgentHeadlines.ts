import * as React from "react";

import {
  getActivityHeadline,
  isMeaningfulItem,
  isSpineItem,
} from "@/features/agents/ui/agentSessionTranscriptPresentation";
import { useAgentTranscript } from "@/features/agents/ui/useObserverEvents";
import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";

function collectHeadlinesWithFilter(
  scopedTranscript: TranscriptItem[],
  passFilter: (item: TranscriptItem) => boolean,
  maxHeadlines: number,
): string[] {
  const seen = new Set<string>();
  const headlines: string[] = [];

  for (let i = scopedTranscript.length - 1; i >= 0; i--) {
    const item = scopedTranscript[i];
    if (!passFilter(item)) {
      continue;
    }
    const headline = getActivityHeadline(item);
    if (!headline || seen.has(headline)) {
      continue;
    }

    seen.add(headline);
    headlines.unshift(headline);
    if (headlines.length >= maxHeadlines) {
      break;
    }
  }

  return headlines;
}

export function collectActivityHeadlines(
  transcript: TranscriptItem[],
  channelId?: string | null,
  maxHeadlines = 5,
): string[] {
  // Prefer reverse scan over filter+some so long transcripts stop early.
  // Only materialize a channel-scoped array when a channel filter is set.
  let scopedTranscript = transcript;
  if (channelId) {
    scopedTranscript = [];
    for (let i = 0; i < transcript.length; i++) {
      const item = transcript[i];
      if (item.channelId === channelId) {
        scopedTranscript.push(item);
      }
    }
  }

  // Prefer spine rows when any exist. Reverse-scan to find one instead of
  // walking the whole array with .some() on long transcripts.
  let hasSpine = false;
  for (let i = scopedTranscript.length - 1; i >= 0; i--) {
    if (isSpineItem(scopedTranscript[i])) {
      hasSpine = true;
      break;
    }
  }

  return collectHeadlinesWithFilter(
    scopedTranscript,
    hasSpine ? isSpineItem : isMeaningfulItem,
    maxHeadlines,
  );
}

export function useWorkingAgentHeadlines(
  enabled: boolean,
  agentPubkey: string | undefined,
  channelId?: string | null,
  maxHeadlines = 5,
): string[] {
  const transcript = useAgentTranscript(enabled, agentPubkey);

  return React.useMemo(
    () => collectActivityHeadlines(transcript, channelId, maxHeadlines),
    [channelId, maxHeadlines, transcript],
  );
}
