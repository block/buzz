import {
  formatTranscriptTimestampTitle,
  getToolDurationDisplay,
} from "../agentSessionUtils";
import {
  ActivityRow,
  ActivityRowLabel,
  splitActivityRowLabel,
} from "./ActivityRow";
import type { ActivityRenderClassItemProps } from "./types";

export function SuppressedActivity(props: ActivityRenderClassItemProps) {
  if (props.item.type !== "tool") {
    return null;
  }

  const action = props.item.descriptor.action;
  const labelParts =
    action ?? splitActivityRowLabel(props.item.descriptor.label);
  const duration = getToolDurationDisplay(props.item);

  return (
    <ActivityRow
      testId="transcript-suppressed-item"
      title={formatTranscriptTimestampTitle(props.item.timestamp)}
    >
      <ActivityRowLabel
        object={labelParts?.object ?? props.item.descriptor.preview}
        openToneScope="none"
        verb={labelParts?.verb ?? props.item.descriptor.label}
      />
      {duration ? (
        <span className="shrink-0 text-xs text-muted-foreground/60">
          {duration}
        </span>
      ) : null}
    </ActivityRow>
  );
}
