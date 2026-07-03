import { Markdown } from "@/shared/ui/markdown";
import {
  ActivityRow,
  ActivityRowContent,
  ActivityRowLabel,
} from "./ActivityRow";
import { ToolActivity } from "./ToolActivity";
import { formatTranscriptTimestampTitle } from "../agentSessionUtils";
import type { ActivityRenderClassItemProps } from "./types";

export function PlanActivity(props: ActivityRenderClassItemProps) {
  if (props.item.type === "tool") {
    return <ToolActivity {...props} />;
  }
  if (props.item.type !== "plan") {
    return null;
  }

  if (props.item.isUpdate) {
    return (
      <ActivityRow
        testId="transcript-plan-update-item"
        title={formatTranscriptTimestampTitle(props.item.timestamp)}
      >
        <ActivityRowLabel
          object={<PlanUpdateLabelObject text={props.item.text} />}
          openToneScope="none"
          verb="Updated"
        />
      </ActivityRow>
    );
  }

  return (
    <ActivityRow
      testId="transcript-plan-item"
      title={formatTranscriptTimestampTitle(props.item.timestamp)}
    >
      <ActivityRowLabel object="plan" openToneScope="tool" verb="Updated" />
      <ActivityRowContent className="pt-1 pb-1.5 text-sm leading-5 text-muted-foreground">
        <Markdown
          className="leading-5"
          content={props.item.text.trim() || "No plan details."}
        />
      </ActivityRowContent>
    </ActivityRow>
  );
}

function PlanUpdateLabelObject({ text }: { text: string }) {
  return (
    <>
      plan
      {text ? (
        <>
          {" · "}
          <span className="text-foreground">{text}</span>
        </>
      ) : null}
    </>
  );
}
