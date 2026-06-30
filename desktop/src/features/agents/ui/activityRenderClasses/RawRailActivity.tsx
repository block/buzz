import { ChevronDown, TerminalSquare } from "lucide-react";

import { ToolActivity } from "./ToolActivity";
import { TranscriptTimestamp } from "./TranscriptTimestamp";
import type { ActivityRenderClassItemProps } from "./types";

export function RawRailActivity(props: ActivityRenderClassItemProps) {
  if (props.item.type === "tool") {
    return <ToolActivity {...props} />;
  }
  if (props.item.type !== "metadata") {
    return null;
  }

  return (
    <details
      className="group not-prose w-full rounded-md border border-border/50 bg-muted/20 px-2 py-1"
      data-testid="transcript-metadata-item"
    >
      <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 py-px text-muted-foreground">
        <TerminalSquare className="h-3.5 w-3.5 shrink-0 opacity-70" />
        <span className="truncate text-xs font-medium">{props.item.title}</span>
        <span className="shrink-0 text-xs text-muted-foreground/70">
          {props.item.sections.length} section
          {props.item.sections.length === 1 ? "" : "s"}
        </span>
        <TranscriptTimestamp timestamp={props.item.timestamp} />
        <ChevronDown className="h-4 w-4 shrink-0 transition-transform group-open:rotate-180" />
      </summary>
      <div className="space-y-3 py-2 pl-5">
        {props.item.sections.map((section) => (
          <details
            className="group/section"
            key={`${section.title}:${section.body.slice(0, 48)}`}
          >
            <summary className="inline-flex max-w-full cursor-pointer list-none items-center gap-1.5 text-xs font-medium text-foreground/80">
              <span className="truncate">{section.title}</span>
              <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-open/section:rotate-180" />
            </summary>
            <pre className="mt-2 max-h-56 overflow-auto whitespace-pre-wrap wrap-break-word rounded-md bg-muted/50 px-3 py-2 font-mono text-xs leading-5 text-muted-foreground">
              {section.body.trim() || "No metadata."}
            </pre>
          </details>
        ))}
      </div>
    </details>
  );
}
