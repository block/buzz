import { CheckCheck, ChevronDown } from "lucide-react";

import { Markdown } from "@/shared/ui/markdown";
import { ToolActivity } from "./ToolActivity";
import { TranscriptTimestamp } from "./TranscriptTimestamp";
import type { ActivityRenderClassItemProps } from "./types";

export function PlanActivity(props: ActivityRenderClassItemProps) {
  if (props.item.type === "tool") {
    return <ToolActivity {...props} />;
  }
  if (props.item.type !== "plan") {
    return null;
  }

  return (
    <details
      className="group not-prose w-full rounded-md border border-primary/15 bg-primary/5 px-2 py-1"
      data-testid="transcript-plan-item"
      open
    >
      <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 py-px text-primary/90">
        <CheckCheck className="h-3.5 w-3.5 shrink-0" />
        <span className="truncate text-xs font-medium">{props.item.title}</span>
        <TranscriptTimestamp timestamp={props.item.timestamp} />
        <ChevronDown className="h-4 w-4 shrink-0 transition-transform group-open:rotate-180" />
      </summary>
      <div className="py-2 pl-5 text-sm leading-6 text-muted-foreground">
        <Markdown
          compact
          content={props.item.text.trim() || "No plan details."}
        />
      </div>
    </details>
  );
}
