import { Sparkles } from "lucide-react";

import type { TriageSuggestion } from "@/features/triage/api";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Markdown } from "@/shared/ui/markdown";

type TriageRowProps = {
  isSelected: boolean;
  onSelect: (eventId: string) => void;
  suggestion: TriageSuggestion;
};

export function TriageRow({
  isSelected,
  onSelect,
  suggestion,
}: TriageRowProps) {
  return (
    <button
      className={cn(
        "w-full border-b border-border/50 px-3 py-2.5 text-left transition-colors",
        isSelected ? "bg-accent/60" : "hover:bg-accent/30",
      )}
      data-testid="triage-row"
      onClick={() => onSelect(suggestion.eventId)}
      type="button"
    >
      <div className="flex min-w-0 items-center gap-2">
        <span className="min-w-0 truncate text-sm font-medium text-foreground">
          {suggestion.authorLabel ?? "Unknown sender"}
        </span>
        {suggestion.channelName ? (
          <Badge variant="outline">#{suggestion.channelName}</Badge>
        ) : null}
        {suggestion.learned ? (
          <span
            className="flex shrink-0 items-center gap-0.5 text-2xs text-primary"
            title="This verdict came from your earlier correction"
          >
            <Sparkles className="h-3 w-3" />
            learned
          </span>
        ) : null}
      </div>

      {suggestion.content.trim() ? (
        <Markdown
          className="mt-1 line-clamp-2 text-sm leading-5 text-muted-foreground"
          content={suggestion.content}
          interactive={false}
        />
      ) : null}

      <p className="mt-1.5 line-clamp-2 text-2xs italic text-muted-foreground/80">
        {suggestion.reason}
      </p>
    </button>
  );
}
