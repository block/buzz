import { Markdown } from "@/shared/ui/markdown";
import {
  ActivityRow,
  ActivityRowContent,
  ActivityRowLabel,
} from "./ActivityRow";
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
    <ActivityRow testId="transcript-plan-item">
      <ActivityRowLabel object="plan" openToneScope="tool" verb="Updated" />
      <TranscriptTimestamp timestamp={props.item.timestamp} />
      <ActivityRowContent className="pt-1 pb-1.5 text-sm leading-6 text-muted-foreground">
        <Markdown
          compact
          content={props.item.text.trim() || "No plan details."}
        />
      </ActivityRowContent>
    </ActivityRow>
  );
}
