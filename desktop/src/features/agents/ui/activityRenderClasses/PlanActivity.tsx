import { Check, ListChecks, Loader } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Markdown } from "@/shared/ui/markdown";
import {
  ActivityRow,
  ActivityRowContent,
  ActivityRowLabel,
} from "./ActivityRow";
import { ToolActivity } from "./ToolActivity";
import {
  formatPlanChecklistProgress,
  parsePlanChecklist,
  type PlanChecklistEntry,
} from "../agentSessionPlanChecklist";
import { useAgentSessionTranscriptVariant } from "../agentSessionTranscriptContext";
import { formatTranscriptTimestampTitle } from "../agentSessionUtils";
import type { ActivityRenderClassItemProps } from "./types";

export function PlanActivity(props: ActivityRenderClassItemProps) {
  if (props.item.type === "tool") {
    return <ToolActivity {...props} />;
  }
  if (props.item.type !== "plan") {
    return null;
  }

  return <PlanItem item={props.item} />;
}

function PlanItem({
  item,
}: {
  item: Extract<ActivityRenderClassItemProps["item"], { type: "plan" }>;
}) {
  const variant = useAgentSessionTranscriptVariant();

  // Plan-update markers stay a one-line "Updated plan · 3/5 complete" row in
  // every variant: in focus mode the card below already carries current state,
  // so the marker is a timeline note and must not compete with it.
  if (item.isUpdate) {
    return (
      <ActivityRow
        testId="transcript-plan-update-item"
        title={formatTranscriptTimestampTitle(item.timestamp)}
      >
        <ActivityRowLabel
          object={<PlanUpdateLabelObject text={item.text} />}
          openToneScope="none"
          verb="Updated"
        />
      </ActivityRow>
    );
  }

  if (variant === "conversation") {
    return <ConversationPlanCard item={item} />;
  }

  return (
    <ActivityRow
      testId="transcript-plan-item"
      title={formatTranscriptTimestampTitle(item.timestamp)}
    >
      <ActivityRowLabel object="plan" openToneScope="tool" verb="Updated" />
      <ActivityRowContent className="pt-1 pb-1.5 text-sm leading-5 text-muted-foreground">
        <Markdown
          className="leading-5"
          content={item.text.trim() || "No plan details."}
        />
      </ActivityRowContent>
    </ActivityRow>
  );
}

/**
 * Focus-mode plan: a checklist card rather than a collapsed markdown row.
 *
 * The plan item is mutated in place by the transcript reducer (`replaceItem`
 * keeps the same item id), so this card is the one surface that shows current
 * plan state — re-rendering it as entries flip is the "updates in place"
 * behavior; no local state is involved.
 *
 * Adapters that send free-form plan text instead of ACP `entries[]` produce no
 * checklist lines, so the card falls back to the markdown body.
 */
function ConversationPlanCard({
  item,
}: {
  item: Extract<ActivityRenderClassItemProps["item"], { type: "plan" }>;
}) {
  const text = item.text.trim();
  const checklist = parsePlanChecklist(text);
  const progress = formatPlanChecklistProgress(checklist);

  return (
    <div
      className="not-prose w-full rounded-xl border border-border/70 bg-card/60 px-4 py-3"
      data-testid="transcript-plan-item"
      data-variant="conversation-plan-card"
      title={formatTranscriptTimestampTitle(item.timestamp)}
    >
      <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <ListChecks aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />
        <span>Plan</span>
        {progress ? (
          <>
            <span aria-hidden="true">·</span>
            <span data-testid="transcript-plan-progress">{progress}</span>
          </>
        ) : null}
      </div>
      {checklist.entries.length > 0 ? (
        <ul className="mt-2 flex flex-col gap-1.5">
          {checklist.entries.map((entry, index) => (
            <PlanChecklistRow
              entry={entry}
              // biome-ignore lint/suspicious/noArrayIndexKey: plan entries have no stable id and are replaced wholesale on each update
              key={index}
            />
          ))}
        </ul>
      ) : (
        <div className="mt-2 text-sm leading-relaxed text-muted-foreground">
          <Markdown
            className="leading-relaxed"
            content={text || "No plan details."}
          />
        </div>
      )}
    </div>
  );
}

function PlanChecklistRow({ entry }: { entry: PlanChecklistEntry }) {
  return (
    <li
      className="flex items-start gap-2 text-sm leading-relaxed"
      data-plan-entry-status={entry.status}
    >
      <span className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center">
        {entry.status === "completed" ? (
          <Check aria-hidden="true" className="h-3.5 w-3.5 text-primary" />
        ) : entry.status === "in_progress" ? (
          <Loader
            aria-hidden="true"
            className="h-3.5 w-3.5 text-muted-foreground"
          />
        ) : (
          <span className="h-3 w-3 rounded-[3px] border border-border" />
        )}
      </span>
      <span
        className={cn(
          entry.status === "completed"
            ? "text-muted-foreground line-through"
            : entry.status === "in_progress"
              ? "text-foreground"
              : "text-muted-foreground",
        )}
      >
        {entry.label}
      </span>
      <span className="sr-only">
        {entry.status === "completed"
          ? " (completed)"
          : entry.status === "in_progress"
            ? " (in progress)"
            : " (pending)"}
      </span>
    </li>
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
