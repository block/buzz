import { AlertCircle, CircleDot } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { ToolActivity } from "./ToolActivity";
import { TranscriptTimestamp } from "./TranscriptTimestamp";
import type { ActivityRenderClassItemProps } from "./types";

export function LifecycleActivity(props: ActivityRenderClassItemProps) {
  if (props.item.type === "tool") {
    return <ToolActivity {...props} />;
  }
  if (props.item.type !== "lifecycle") {
    return null;
  }

  const isError =
    props.item.renderClass === "error" ||
    props.item.title.toLowerCase().includes("error");

  return (
    <div
      className={cn(
        "flex items-center justify-start gap-1.5 rounded-md px-2 py-1.5 text-left text-xs",
        isError
          ? "border border-destructive/20 bg-destructive/5 text-destructive"
          : "text-muted-foreground/80",
      )}
      data-testid="transcript-lifecycle-item"
    >
      {isError ? (
        <AlertCircle className="h-3.5 w-3.5 shrink-0" />
      ) : (
        <CircleDot className="h-3 w-3 shrink-0 opacity-50" />
      )}
      <span className="font-medium">{props.item.title}</span>
      {props.item.text ? (
        <span className="opacity-80">· {props.item.text}</span>
      ) : null}
      <TranscriptTimestamp timestamp={props.item.timestamp} />
    </div>
  );
}
