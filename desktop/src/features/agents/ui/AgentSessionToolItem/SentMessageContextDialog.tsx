import * as React from "react";
import { CheckCheck, ChevronDown } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { formatCodeValue } from "../agentSessionUtils";

export function SentMessageContextDialog({
  args,
  description,
  duration,
  hasArgs,
  hasResult,
  isError,
  label,
  onOpenChange,
  open,
  preview,
  result,
}: {
  args: Record<string, unknown>;
  description?: string;
  duration: string | null;
  hasArgs: boolean;
  hasResult: boolean;
  isError: boolean;
  label: string;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  preview: string | null;
  result: string;
}) {
  const sections = buildSentMessageContextSections({
    args,
    description,
    hasArgs,
    hasResult,
    isError,
    preview,
    result,
  });

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="px-6 pb-3 pt-5 pr-14">
            <DialogTitle>Sent message context</DialogTitle>
            <DialogDescription className="flex items-center gap-1.5">
              <CheckCheck className="h-3.5 w-3.5 shrink-0" />
              <span>{label}</span>
              {duration ? <span className="shrink-0">{duration}</span> : null}
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-6 pt-2">
            <SentMessageContextSections sections={sections} />
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

type SentMessageContextSection = {
  body: string;
  title: string;
};

function buildSentMessageContextSections({
  args,
  description,
  hasArgs,
  hasResult,
  isError,
  preview,
  result,
}: {
  args: Record<string, unknown>;
  description?: string;
  hasArgs: boolean;
  hasResult: boolean;
  isError: boolean;
  preview: string | null;
  result: string;
}): SentMessageContextSection[] {
  const sections: SentMessageContextSection[] = [];
  if (preview) {
    sections.push({ title: "Message", body: preview });
  }
  if (description) {
    sections.push({ title: "Tool", body: description });
  }
  if (hasArgs) {
    sections.push({
      title: "Parameters",
      body: JSON.stringify(args, null, 2),
    });
  }
  if (hasResult) {
    sections.push({
      title: isError ? "Error" : "Result",
      body: formatCodeValue(result),
    });
  }
  if (sections.length === 0) {
    sections.push({
      title: "Status",
      body: "Waiting for tool details.",
    });
  }
  return sections;
}

function SentMessageContextSections({
  sections,
}: {
  sections: SentMessageContextSection[];
}) {
  return (
    <div className="space-y-3" data-testid="transcript-sent-message-context">
      {sections.map((section) => (
        <SentMessageContextSectionAccordion
          key={`${section.title}:${section.body.slice(0, 48)}`}
          section={section}
        />
      ))}
    </div>
  );
}

function SentMessageContextSectionAccordion({
  section,
}: {
  section: SentMessageContextSection;
}) {
  const [open, setOpen] = React.useState(false);
  const body = section.body.trim();

  return (
    <article className="overflow-hidden rounded-2xl bg-muted/40">
      <button
        aria-expanded={open}
        className="w-full px-4 py-3 text-left transition-colors hover:bg-muted/50"
        onClick={() => setOpen((value) => !value)}
        type="button"
      >
        <div className="flex items-start gap-3">
          <div className="min-w-0 flex-1">
            <div
              className={cn(
                "text-sm font-semibold text-foreground",
                !open && "line-clamp-2",
              )}
            >
              {section.title}
            </div>
            <div
              className={cn(
                "mt-1 whitespace-pre-wrap break-all text-xs leading-5 text-foreground/70",
                !open && "line-clamp-2",
              )}
            >
              {body.length > 0 ? (
                body
              ) : (
                <span className="italic text-foreground/50">No metadata.</span>
              )}
            </div>
          </div>
          <ChevronDown
            className={cn(
              "mt-0.5 h-4 w-4 shrink-0 text-muted-foreground transition-transform",
              open && "rotate-180",
            )}
          />
        </div>
      </button>
    </article>
  );
}
