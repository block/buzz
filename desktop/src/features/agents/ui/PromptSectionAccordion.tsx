import React from "react";
import { ChevronDown } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import type { PromptSection } from "./agentSessionTypes";

export function PromptSectionList({
  className,
  sections,
}: {
  className?: string;
  sections: PromptSection[];
}) {
  return (
    <div
      className={cn("space-y-2", className)}
      data-testid="transcript-prompt-context-sections"
    >
      {sections.map((section) => (
        <PromptSectionAccordion
          key={`${section.title}:${section.body.slice(0, 48)}`}
          section={section}
        />
      ))}
    </div>
  );
}

export function PromptSectionAccordion({
  section,
}: {
  section: PromptSection;
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
                !open && "line-clamp-2 wrap-break-word",
              )}
            >
              {section.title}
            </div>
            <div
              className={cn(
                "mt-1 text-xs leading-5 text-foreground/70",
                open
                  ? "whitespace-pre-wrap wrap-break-word"
                  : "line-clamp-2 wrap-break-word",
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
