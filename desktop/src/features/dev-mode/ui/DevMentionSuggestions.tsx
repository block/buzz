import { useAuthorColorResolver } from "@/features/dev-mode/lib/authorColors";
import type { MentionSuggestion } from "@/features/dev-mode/lib/useMentionAutocomplete";
import { cn } from "@/shared/lib/cn";

/**
 * `@user` autocomplete popup anchored above a composer textarea. Mouse
 * clicks accept without stealing focus from the textarea (mousedown is
 * prevented), matching the keyboard-first composer conventions.
 */
export function DevMentionSuggestions({
  suggestions,
  selectedIndex,
  onAccept,
}: {
  suggestions: MentionSuggestion[];
  selectedIndex: number;
  onAccept: (suggestion: MentionSuggestion) => void;
}) {
  const resolveColor = useAuthorColorResolver();
  return (
    <div
      className="absolute bottom-full left-0 z-10 mb-1 min-w-64 max-w-full border border-border bg-background font-mono text-sm shadow-md"
      data-testid="dev-mode-mention-suggestions"
    >
      <div className="border-b border-border/60 px-2 py-0.5 text-xs text-muted-foreground">
        tag someone · tab/enter: insert · esc: dismiss
      </div>
      {suggestions.map((suggestion, index) => (
        <button
          key={suggestion.pubkey}
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
          <span className="select-none text-muted-foreground/60">@</span>
          <span
            className="min-w-0 truncate"
            style={{ color: resolveColor(suggestion.pubkey) }}
          >
            {suggestion.name}
          </span>
          {suggestion.isAgent ? (
            <span className="ml-auto shrink-0 select-none text-xs text-muted-foreground/60">
              agent
            </span>
          ) : !suggestion.isMember ? (
            <span className="ml-auto shrink-0 select-none text-xs text-muted-foreground/60">
              adds to channel
            </span>
          ) : null}
        </button>
      ))}
    </div>
  );
}
