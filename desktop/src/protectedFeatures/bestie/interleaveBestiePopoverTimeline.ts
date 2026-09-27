import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";
import type { TimelineMessage } from "@/features/messages/types";

export type BestiePopoverTimelineRun =
  | { kind: "messages"; messages: TimelineMessage[] }
  | { kind: "activity"; items: TranscriptItem[] };

type SortedEntry =
  | {
      kind: "message";
      id: string;
      sortAt: number;
      message: TimelineMessage;
    }
  | {
      kind: "activity";
      id: string;
      sortAt: number;
      item: TranscriptItem;
    };

function messageSortAt(message: TimelineMessage): number {
  // TimelineMessage.createdAt is Nostr unix seconds.
  return message.createdAt * 1000;
}

function activitySortAt(item: TranscriptItem): number {
  const parsed = Date.parse(item.timestamp);
  return Number.isFinite(parsed) ? parsed : 0;
}

/**
 * Merge chat bubbles and ACP activity into chronological runs for the Bestie
 * popover. Contiguous same-kind items stay grouped so message lists and the
 * activity transcript list can each render a run.
 *
 * On equal timestamps, activity sorts before messages so thinking/tool rows
 * appear ahead of the reply bubble they produced (Claude Code–style).
 */
export function buildBestiePopoverTimelineRuns(
  messages: readonly TimelineMessage[],
  activityItems: readonly TranscriptItem[],
): BestiePopoverTimelineRun[] {
  const entries: SortedEntry[] = [
    ...messages.map((message) => ({
      kind: "message" as const,
      id: message.id,
      sortAt: messageSortAt(message),
      message,
    })),
    ...activityItems.map((item) => ({
      kind: "activity" as const,
      id: item.id,
      sortAt: activitySortAt(item),
      item,
    })),
  ];

  entries.sort((left, right) => {
    if (left.sortAt !== right.sortAt) {
      return left.sortAt - right.sortAt;
    }
    if (left.kind !== right.kind) {
      return left.kind === "activity" ? -1 : 1;
    }
    return left.id.localeCompare(right.id);
  });

  const runs: BestiePopoverTimelineRun[] = [];
  for (const entry of entries) {
    const last = runs.at(-1);
    if (entry.kind === "message") {
      if (last?.kind === "messages") {
        last.messages.push(entry.message);
      } else {
        runs.push({ kind: "messages", messages: [entry.message] });
      }
      continue;
    }
    if (last?.kind === "activity") {
      last.items.push(entry.item);
    } else {
      runs.push({ kind: "activity", items: [entry.item] });
    }
  }
  return runs;
}
