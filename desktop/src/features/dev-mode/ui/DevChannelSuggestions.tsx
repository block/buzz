import type { ChannelRef } from "@/features/dev-mode/lib/channelRefs";
import { cn } from "@/shared/lib/cn";

/**
 * `#channel` autocomplete popup anchored above a composer textarea. Mouse
 * clicks accept without stealing focus from the textarea (mousedown is
 * prevented), matching the keyboard-first composer conventions.
 */
export function DevChannelSuggestions({
  suggestions,
  selectedIndex,
  onAccept,
}: {
  suggestions: ChannelRef[];
  selectedIndex: number;
  onAccept: (suggestion: ChannelRef) => void;
}) {
  return (
    <div
      className="absolute bottom-full left-0 z-10 mb-1 min-w-64 max-w-full border border-border bg-background font-mono text-sm shadow-md"
      data-testid="dev-mode-channel-suggestions"
    >
      <div className="border-b border-border/60 px-2 py-0.5 text-xs text-muted-foreground">
        link a channel · tab/enter: insert · esc: dismiss
      </div>
      {suggestions.map((suggestion, index) => (
        <button
          key={suggestion.id}
          className={cn(
            "flex w-full cursor-pointer items-baseline gap-1 px-2 py-0.5 text-left",
            index === selectedIndex
              ? "bg-muted text-foreground"
              : "text-muted-foreground hover:bg-muted/50",
          )}
          onClick={() => onAccept(suggestion)}
          onMouseDown={(event) => event.preventDefault()}
          type="button"
        >
          <span className="select-none text-muted-foreground/60">#</span>
          <span className="min-w-0 truncate">{suggestion.name}</span>
        </button>
      ))}
    </div>
  );
}
