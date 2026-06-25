import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";
import { cn } from "@/shared/lib/cn";
import { Skeleton } from "@/shared/ui/skeleton";

const TIMELINE_SKELETON_CACHE_PREFIX = "buzz-timeline-skeleton-shape.v1";
const timelineSkeletonWidthClasses = [
  "w-10",
  "w-12",
  "w-16",
  "w-20",
  "w-24",
  "w-28",
  "w-32",
  "w-1/2",
  "w-2/3",
  "w-3/4",
  "w-4/5",
  "w-5/6",
  "w-11/12",
  "w-full",
] as const;
const timelineSkeletonActionKeys = ["reply", "react", "more"] as const;
const timelineSkeletonBodyLineKeys = ["primary", "secondary", "tertiary"];

type TimelineSkeletonWidthClass = (typeof timelineSkeletonWidthClasses)[number];

export type TimelineSkeletonRowShape = {
  actionCount: number;
  authorWidthClass: TimelineSkeletonWidthClass;
  bodyLineWidthClasses: TimelineSkeletonWidthClass[];
  key: string;
};

type TimelineSkeletonCachePayload = {
  rows: TimelineSkeletonRowShape[];
  version: 1;
};

const fallbackTimelineSkeletonRows: TimelineSkeletonRowShape[] = [
  {
    actionCount: 3,
    authorWidthClass: "w-28",
    bodyLineWidthClasses: ["w-full", "w-4/5"],
    key: "first",
  },
  {
    actionCount: 3,
    authorWidthClass: "w-24",
    bodyLineWidthClasses: ["w-full", "w-2/3"],
    key: "second",
  },
  {
    actionCount: 3,
    authorWidthClass: "w-32",
    bodyLineWidthClasses: ["w-5/6"],
    key: "third",
  },
  {
    actionCount: 3,
    authorWidthClass: "w-20",
    bodyLineWidthClasses: ["w-full", "w-4/5"],
    key: "fourth",
  },
];

function isTimelineWidthClass(
  value: unknown,
): value is TimelineSkeletonWidthClass {
  return timelineSkeletonWidthClasses.includes(
    value as TimelineSkeletonWidthClass,
  );
}

function parseTimelineSkeletonRows(
  rows: unknown,
): TimelineSkeletonRowShape[] | null {
  if (!Array.isArray(rows)) return null;

  const parsed = rows
    .slice(0, 4)
    .filter((row: unknown): row is TimelineSkeletonRowShape => {
      if (typeof row !== "object" || row === null) return false;
      const record = row as Record<string, unknown>;
      return (
        typeof record.key === "string" &&
        Number.isInteger(record.actionCount) &&
        Number(record.actionCount) >= 0 &&
        Number(record.actionCount) <= timelineSkeletonActionKeys.length &&
        isTimelineWidthClass(record.authorWidthClass) &&
        Array.isArray(record.bodyLineWidthClasses) &&
        record.bodyLineWidthClasses.length > 0 &&
        record.bodyLineWidthClasses.length <= 3 &&
        record.bodyLineWidthClasses.every(isTimelineWidthClass)
      );
    });

  return parsed.length > 0 ? parsed : null;
}

function timelineSkeletonCacheKey(channelId?: string | null) {
  return channelId ? `${TIMELINE_SKELETON_CACHE_PREFIX}:${channelId}` : null;
}

function readTimelineSkeletonRows(
  channelId?: string | null,
): TimelineSkeletonRowShape[] | null {
  const cacheKey = timelineSkeletonCacheKey(channelId);
  if (!cacheKey || typeof window === "undefined") return null;

  try {
    const raw = window.localStorage.getItem(cacheKey);
    if (!raw) return null;

    const parsed = JSON.parse(raw) as unknown;
    if (typeof parsed !== "object" || parsed === null) return null;
    const payload = parsed as Record<string, unknown>;
    if (payload.version !== 1) return null;

    return parseTimelineSkeletonRows(payload.rows);
  } catch {
    return null;
  }
}

function writeTimelineSkeletonRows(
  channelId: string | null | undefined,
  rows: TimelineSkeletonRowShape[],
) {
  const cacheKey = timelineSkeletonCacheKey(channelId);
  if (!cacheKey || rows.length === 0 || typeof window === "undefined") return;

  const payload: TimelineSkeletonCachePayload = {
    rows: rows.slice(0, 4),
    version: 1,
  };

  try {
    window.localStorage.setItem(cacheKey, JSON.stringify(payload));
  } catch {
    // localStorage can be unavailable or full in embedded webviews.
  }
}

function widthClassForText(text: string): TimelineSkeletonWidthClass {
  const length = text.trim().length;
  if (length >= 18) return "w-32";
  if (length >= 14) return "w-28";
  if (length >= 10) return "w-24";
  if (length >= 7) return "w-20";
  return "w-16";
}

function bodyLineWidthsForText(text: string): TimelineSkeletonWidthClass[] {
  const length = text.replace(/\s+/g, " ").trim().length;
  if (length >= 140) return ["w-full", "w-11/12", "w-2/3"];
  if (length >= 90) return ["w-full", "w-4/5"];
  if (length >= 48) return ["w-5/6", "w-2/3"];
  if (length >= 24) return ["w-2/3"];
  return ["w-1/2"];
}

function rowsFromMessages(
  messages: TimelineMessage[],
): TimelineSkeletonRowShape[] {
  return messages.slice(-4).map((message, index) => ({
    actionCount: Math.min(
      timelineSkeletonActionKeys.length,
      Math.max(2, (message.reactions?.length ?? 0) + 2),
    ),
    authorWidthClass: widthClassForText(message.author),
    bodyLineWidthClasses: bodyLineWidthsForText(message.body),
    key: message.renderKey ?? message.id ?? `message-${index}`,
  }));
}

export function useTimelineSkeletonRows({
  channelId,
  isLoading,
  messages,
}: {
  channelId?: string | null;
  isLoading: boolean;
  messages: TimelineMessage[];
}) {
  const liveRows = React.useMemo(() => rowsFromMessages(messages), [messages]);
  const cachedRows = React.useMemo(
    () => readTimelineSkeletonRows(channelId),
    [channelId],
  );

  React.useEffect(() => {
    if (isLoading || liveRows.length === 0) return;
    writeTimelineSkeletonRows(channelId, liveRows);
  }, [channelId, isLoading, liveRows]);

  return liveRows.length > 0 ? liveRows : (cachedRows ?? undefined);
}

type TimelineSkeletonProps = {
  rows?: TimelineSkeletonRowShape[];
};

export function TimelineSkeleton({ rows }: TimelineSkeletonProps) {
  const skeletonRows = rows?.length ? rows : fallbackTimelineSkeletonRows;

  return (
    <>
      {skeletonRows.map((row) => (
        <article
          className="relative mx-1 flex items-start gap-2.5 rounded-2xl px-2 py-2"
          key={row.key}
        >
          <Skeleton className="h-9 w-9 shrink-0 rounded-full" />
          <div className="-mt-1 min-w-0 flex-1">
            <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0">
              <Skeleton className={cn("h-[15px]", row.authorWidthClass)} />
              <Skeleton className="h-3 w-10" />
            </div>
            <div className="mt-1 space-y-1.5 pb-2">
              {timelineSkeletonBodyLineKeys
                .slice(0, row.bodyLineWidthClasses.length)
                .map((lineKey, index) => (
                  <Skeleton
                    className={cn("h-4", row.bodyLineWidthClasses[index])}
                    key={`${row.key}-body-${lineKey}`}
                  />
                ))}
            </div>
            <div className="flex items-center gap-4">
              {timelineSkeletonActionKeys
                .slice(0, row.actionCount)
                .map((key) => (
                  <Skeleton className="h-4 w-8 rounded-full" key={key} />
                ))}
            </div>
          </div>
        </article>
      ))}
    </>
  );
}
