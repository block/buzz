import * as React from "react";
import { ChevronDown } from "lucide-react";

import { cn } from "@/shared/lib/cn";

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
  defaultOpen?: boolean;
  openToneScope?: Exclude<ActivityRowToneScope, "none">;
  testId?: string;
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
  defaultOpen = false,
  openToneScope = "tool",
  testId,
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
      open={defaultOpen}
    >
      <summary
        className={cn(
          "inline-flex min-h-6 max-w-full cursor-pointer list-none items-center gap-1.5 text-muted-foreground",
          openToneScope === "summary"
            ? "group-open/summary:text-foreground"
            : "group-open:text-foreground",
        )}
      >
        {summaryChildren}
        <ChevronDown
          className={cn(
            "h-3.5 w-3.5 shrink-0 text-muted-foreground/60 transition-transform",
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
  return (
    <span
      className={cn("inline-flex min-w-0 items-center gap-1.5", className)}
      title={title}
    >
      <span
        className={cn(
          "shrink-0 text-sm font-semibold text-muted-foreground/50",
          openToneScope === "none"
            ? null
            : openToneScope === "summary"
              ? "group-open/summary:text-foreground"
              : "group-open:text-foreground",
        )}
      >
        {verb}
      </span>
      {object ? (
        <span
          className={cn(
            "min-w-0 truncate text-sm font-normal text-muted-foreground/60",
            openToneScope === "none"
              ? null
              : openToneScope === "summary"
                ? "group-open/summary:text-foreground"
                : "group-open:text-foreground",
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
    <>
      <span className="shrink-0 text-xs font-semibold text-green-600 dark:text-green-400">
        +{stats.additions}
      </span>
      <span className="shrink-0 text-xs font-semibold text-red-500 dark:text-red-400">
        -{stats.deletions}
      </span>
    </>
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
