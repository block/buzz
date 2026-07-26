import * as React from "react";

import {
  getActivityHeadline,
  isMeaningfulItem,
  isSpineItem,
} from "@/features/agents/ui/agentSessionTranscriptPresentation";
import { useAgentTranscript } from "@/features/agents/ui/useObserverEvents";
import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";

export function collectActivityHeadlines(
  transcript: TranscriptItem[],
  channelId?: string | null,
  maxHeadlines = 5,
): string[] {
  const seen = new Set<string>();
  const headlines: string[] = [];
  const scopedTranscript = channelId
    ? transcript.filter((item) => item.channelId === channelId)
    : transcript;

  const passFilter: (item: TranscriptItem) => boolean = scopedTranscript.some(
    isSpineItem,
  )
    ? isSpineItem
    : isMeaningfulItem;

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
