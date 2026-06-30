import { Markdown } from "@/shared/ui/markdown";
import {
  ActivityRow,
  ActivityRowContent,
  ActivityRowLabel,
} from "./ActivityRow";
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
    <ActivityRow testId="transcript-thought-item">
      <ActivityRowLabel openToneScope="tool" verb={props.item.title} />
      <TranscriptTimestamp timestamp={props.item.timestamp} />
      <ActivityRowContent className="pt-1 pb-1.5 text-sm leading-6 text-muted-foreground">
        <Markdown compact content={props.item.text.trim() || " "} />
      </ActivityRowContent>
    </ActivityRow>
  );
}
