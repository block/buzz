import { Markdown } from "@/shared/ui/markdown";
import {
  ActivityRow,
  ActivityRowContent,
  ActivityRowLabel,
} from "./ActivityRow";
import { ToolActivity } from "./ToolActivity";
import { formatTranscriptTimestampTitle } from "../agentSessionUtils";
import type { ActivityRenderClassItemProps } from "./types";

export function ThoughtActivity(props: ActivityRenderClassItemProps) {
  if (props.item.type === "tool") {
    return <ToolActivity {...props} />;
  }
  if (props.item.type !== "thought") {
    return null;
  }

  return <ThoughtItem item={props.item} />;
}

/**
 * The `default`/`compactPreview` thought row.
 *
 * The `conversation` variant does not reach this presenter: focus mode renders
 * reasoning as a row on the work block's rail (`AgentSessionWorkBlock`), so the
 * separate per-thought disclosure that used to live here is gone rather than
 * duplicated. Its "Thought for Ns" label logic moved with it.
 */
function ThoughtItem({
  item,
}: {
  item: Extract<ActivityRenderClassItemProps["item"], { type: "thought" }>;
}) {
  return (
    <ActivityRow
      testId="transcript-thought-item"
      title={formatTranscriptTimestampTitle(item.timestamp)}
    >
      <ActivityRowLabel openToneScope="tool" verb={item.title} />
      <ActivityRowContent className="pt-1 pb-1.5 text-sm leading-5 text-muted-foreground">
        <Markdown className="leading-5" content={item.text.trim() || " "} />
      </ActivityRowContent>
    </ActivityRow>
  );
}
