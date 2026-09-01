import { ChevronDown } from "lucide-react";

import { cn } from "@/shared/lib/cn";

export function AgentAdvancedFieldsToggle({
  expanded,
  required,
  onToggle,
}: {
  expanded: boolean;
  required: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      aria-expanded={expanded}
      className="inline-flex h-9 items-center gap-1.5 text-sm font-medium text-foreground transition-colors hover:text-foreground/80 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      onClick={onToggle}
      type="button"
    >
      <span>Advanced</span>
      {required ? (
        <span
          aria-hidden="true"
          className="rounded-full bg-destructive/10 px-2 py-0.5 text-xs text-destructive"
          data-testid="persona-advanced-required-badge"
        >
          Required
        </span>
      ) : null}
      <ChevronDown
        className={cn(
          "h-4 w-4 text-muted-foreground transition-transform duration-150 ease-out",
          expanded && "rotate-180",
        )}
      />
    </button>
  );
}
