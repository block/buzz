import * as React from "react";
import { CheckCheck } from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

import {
  formatTurnSetupLabel,
  turnSetupDetail,
} from "./agentSessionTranscriptGrouping";
import type { PromptSection, TranscriptItem } from "./agentSessionTypes";
import { PromptSectionList } from "./PromptSectionAccordion";

type SetupLifecycle = Extract<TranscriptItem, { type: "lifecycle" }>;

/**
 * Same "Prompt context" dialog the activity-feed CheckCheck toggle opens
 * (`AgentSessionTranscriptList` / turn setup footer). Shared so chat reply
 * chrome can reuse it for the matching harness turn.
 */
export function PromptContextDialog({
  onOpenChange,
  open,
  sections,
  setup = [],
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
  sections: PromptSection[];
  setup?: SetupLifecycle[];
}) {
  if (!open) {
    return null;
  }

  const setupText = formatPromptSetupSummary(setup);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-xl overflow-hidden p-0">
        <div className="flex min-w-0 max-h-[85vh] flex-col">
          <DialogHeader className="px-6 pb-3 pt-5 pr-14">
            <DialogTitle>Prompt context</DialogTitle>
            {setupText ? (
              <div className="flex items-center gap-1.5">
                <CheckCheck className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                <DialogDescription>{setupText}</DialogDescription>
              </div>
            ) : null}
          </DialogHeader>
          <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-6 pt-2">
            {sections.length > 0 ? (
              <PromptSectionList sections={sections} />
            ) : (
              <p className="text-sm text-muted-foreground">
                No prompt context recorded for this turn.
              </p>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function formatPromptSetupSummary(items: SetupLifecycle[]) {
  const label = formatTurnSetupLabel(items);
  const detail = turnSetupDetail(items);
  return [label, detail].filter(Boolean).join(" · ");
}
