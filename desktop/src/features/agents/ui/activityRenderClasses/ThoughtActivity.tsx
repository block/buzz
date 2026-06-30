import { Brain, ChevronDown } from "lucide-react";

import { Markdown } from "@/shared/ui/markdown";
import { ToolActivity } from "./ToolActivity";
import { TranscriptTimestamp } from "./TranscriptTimestamp";
import type { ActivityRenderClassItemProps } from "./types";

export function ThoughtActivity(props: ActivityRenderClassItemProps) {
  if (props.item.type === "tool") {
    return <ToolActivity {...props} />;
  }
  if (props.item.type !== "thought") {
    return null;
  }

  return (
    <details
      className="group not-prose w-full rounded-md border border-transparent px-0"
      data-testid="transcript-thought-item"
    >
      <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 py-px text-muted-foreground">
        <Brain className="h-4 w-4" />
        <span className="truncate text-sm font-medium">{props.item.title}</span>
        <TranscriptTimestamp timestamp={props.item.timestamp} />
        <ChevronDown className="h-4 w-4 shrink-0 transition-transform group-open:rotate-180" />
      </summary>
      <div className="py-2 pl-5 text-sm leading-6 text-muted-foreground">
        <Markdown compact content={props.item.text.trim() || " "} />
      </div>
    </details>
  );
}
