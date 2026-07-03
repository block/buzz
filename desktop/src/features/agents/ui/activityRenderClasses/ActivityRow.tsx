import * as React from "react";
import { ChevronDown } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { useAgentSessionTranscriptVariant } from "../agentSessionTranscriptContext";

export type ActivityRowLabelParts = {
  verb: string;
  object?: React.ReactNode;
};

export type ActivityRowStats = {
  additions: number;
  deletions: number;
};

export type ActivityRowToneScope = "none" | "tool" | "summary";

type ActivityRowProps = {
  children: React.ReactNode;
  className?: string;
  openToneScope?: Exclude<ActivityRowToneScope, "none">;
  testId?: string;
  title?: string;
};

type ActivityRowContentProps = {
  children: React.ReactNode;
  className?: string;
};

const ACTIVITY_ROW_CONTENT_MARKER = Symbol("ActivityRowContent");

type ActivityRowContentComponent = React.FC<ActivityRowContentProps> & {
  marker: typeof ACTIVITY_ROW_CONTENT_MARKER;
};

export function ActivityRow({
  children,
  className,
  openToneScope = "tool",
  testId,
  title,
}: ActivityRowProps) {
  const childArray = React.Children.toArray(children);
  const summaryChildren = childArray.filter(
    (child) => !isActivityRowContent(child),
  );
  const contentChildren = childArray.filter(isActivityRowContent);

  if (contentChildren.length === 0) {
    return (
      <div
        className={cn("not-prose flex min-h-6 items-center gap-1.5", className)}
        data-testid={testId}
        title={title}
      >
        {children}
      </div>
    );
  }

  return (
    <details
      className={cn(
        openToneScope === "summary" ? "group/summary" : "group",
        "not-prose w-full",
        className,
      )}
      data-testid={testId}
      title={title}
    >
      <summary
        className={cn(
          "group/row flex min-h-6 w-full max-w-full cursor-pointer list-none items-center gap-1.5 text-muted-foreground",
          openToneScope === "summary"
            ? "group-open/summary:text-foreground"
            : "group-open:text-foreground",
        )}
      >
        {summaryChildren}
        <ChevronDown
          className={cn(
            "h-3.5 w-3.5 shrink-0 text-muted-foreground/60 transition-transform group-hover/row:text-foreground",
            openToneScope === "summary"
              ? "group-open/summary:rotate-180 group-open/summary:text-foreground"
              : "group-open:rotate-180 group-open:text-foreground",
          )}
        />
      </summary>
      {contentChildren.map((child, index) => (
        <div
          className={child.props.className}
          // biome-ignore lint/suspicious/noArrayIndexKey: content regions are static children
          key={index}
        >
          {child.props.children}
        </div>
      ))}
    </details>
  );
}

export function ActivityRowLabel({
  className,
  object,
  openToneScope,
  stats,
  title,
  verb,
}: ActivityRowLabelParts & {
  className?: string;
  openToneScope: ActivityRowToneScope;
  stats?: ActivityRowStats | null;
  title?: string;
}) {
  const variant = useAgentSessionTranscriptVariant();
  const isCompactPreview = variant === "compactPreview";

  return (
    <span
      className={cn("inline-flex min-w-0 items-center gap-1.5", className)}
      title={title}
    >
      <span
        className={cn(
          "shrink-0 font-semibold text-muted-foreground/50",
          isCompactPreview ? "text-xs" : "text-sm",
          openToneScope === "none"
            ? null
            : openToneScope === "summary"
              ? "transition-colors group-hover/row:text-foreground group-open/summary:text-foreground"
              : "transition-colors group-hover/row:text-foreground group-open:text-foreground",
        )}
      >
        {verb}
      </span>
      {object ? (
        <span
          className={cn(
            "min-w-0 truncate font-normal text-muted-foreground/60",
            isCompactPreview ? "text-xs" : "text-sm",
            openToneScope === "none"
              ? null
              : openToneScope === "summary"
                ? "transition-colors group-hover/row:text-foreground group-open/summary:text-foreground"
                : "transition-colors group-hover/row:text-foreground group-open:text-foreground",
          )}
        >
          {object}
        </span>
      ) : null}
      {stats ? <ActivityRowStatsView stats={stats} /> : null}
    </span>
  );
}

export const ActivityRowContent = (({ children }: ActivityRowContentProps) => (
  <>{children}</>
)) as ActivityRowContentComponent;
ActivityRowContent.marker = ACTIVITY_ROW_CONTENT_MARKER;

function ActivityRowStatsView({ stats }: { stats: ActivityRowStats }) {
  return (
    <span className="inline-flex shrink-0 items-center gap-1 text-xs font-semibold leading-5 tabular-nums">
      <span className="text-status-added">+{stats.additions}</span>
      <span className="text-status-deleted">-{stats.deletions}</span>
    </span>
  );
}

function isActivityRowContent(
  child: React.ReactNode,
): child is React.ReactElement<
  ActivityRowContentProps,
  ActivityRowContentComponent
> {
  return (
    React.isValidElement(child) &&
    typeof child.type !== "string" &&
    "marker" in child.type &&
    child.type.marker === ACTIVITY_ROW_CONTENT_MARKER
  );
}

export function splitActivityRowLabel(
  label: string,
): ActivityRowLabelParts | null {
  const match = label.match(
    /^(Added|Archived|Captured|Checked|Compacted|Created|Deleted|Edited|Ran|Read|Removed|Searched|Sent|Unarchived|Updated|Viewed)\s+(.+)$/,
  );
  return match ? { verb: match[1], object: match[2] } : null;
}
