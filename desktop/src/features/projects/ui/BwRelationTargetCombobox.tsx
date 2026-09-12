import * as React from "react";

import {
  bwRelationTargetCandidates,
  filterBwRelationTargets,
  type BwSnapshot,
} from "@/features/projects/bwProjection";
import { cn } from "@/shared/lib/cn";
import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";

function stateLabel(state: string): string {
  return state
    .split("-")
    .map((part) => `${part.slice(0, 1).toUpperCase()}${part.slice(1)}`)
    .join(" ");
}

/** Search/select an enrolled issue by its visible ISS number, title, full id,
 * or current state. The selected value remains the canonical 64-char root id
 * expected by NIP-BW. */
export function BwRelationTargetCombobox({
  currentIssueId,
  disabled,
  onChange,
  snapshot,
  value,
}: {
  currentIssueId: string;
  disabled: boolean;
  onChange: (value: string) => void;
  snapshot: BwSnapshot;
  value: string;
}) {
  const [open, setOpen] = React.useState(false);
  const [highlightedIndex, setHighlightedIndex] = React.useState(0);
  const candidates = bwRelationTargetCandidates(snapshot, currentIssueId);
  const filtered = filterBwRelationTargets(candidates, value);
  const listId = `bw-relation-target-options-${currentIssueId.slice(0, 8)}`;

  const selectTarget = (id: string) => {
    onChange(id);
    setOpen(false);
    setHighlightedIndex(0);
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Escape") {
      setOpen(false);
      return;
    }
    if (filtered.length === 0) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      setOpen(true);
      setHighlightedIndex((index) => {
        const delta = event.key === "ArrowDown" ? 1 : -1;
        return (index + delta + filtered.length) % filtered.length;
      });
      return;
    }
    if (event.key === "Enter" && open) {
      const selected = filtered[highlightedIndex];
      if (selected) {
        event.preventDefault();
        selectTarget(selected.id);
      }
    }
  };

  return (
    <Popover modal={false} onOpenChange={setOpen} open={open}>
      <PopoverAnchor asChild>
        <input
          aria-autocomplete="list"
          aria-controls={listId}
          aria-expanded={open}
          className="h-8 flex-1 rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
          data-testid="bw-relation-target"
          disabled={disabled}
          onChange={(event) => {
            onChange(event.target.value);
            setHighlightedIndex(0);
            setOpen(Boolean(event.target.value.trim()));
          }}
          onFocus={() => setOpen(Boolean(value.trim()))}
          onKeyDown={handleKeyDown}
          placeholder="Target issue number"
          role="combobox"
          spellCheck={false}
          value={value}
        />
      </PopoverAnchor>
      <PopoverContent
        align="start"
        className="w-(--radix-popover-anchor-width) p-1"
        id={listId}
        onOpenAutoFocus={(event) => event.preventDefault()}
        role="listbox"
      >
        <div className="max-h-60 overflow-y-auto">
          {filtered.length === 0 ? (
            <p className="px-3 py-4 text-center text-xs text-muted-foreground">
              No matching issue in this repository.
            </p>
          ) : (
            filtered.map((candidate, index) => (
              <button
                aria-selected={candidate.id === value.trim().toLowerCase()}
                className={cn(
                  "flex w-full items-start justify-between gap-3 rounded-lg px-2 py-1.5 text-left text-xs transition-colors hover:bg-accent hover:text-accent-foreground",
                  index === highlightedIndex &&
                    "bg-accent text-accent-foreground",
                )}
                data-testid="bw-relation-target-option"
                key={candidate.id}
                onClick={() => selectTarget(candidate.id)}
                onMouseEnter={() => setHighlightedIndex(index)}
                role="option"
                type="button"
              >
                <span className="min-w-0">
                  <span className="block font-mono font-medium">
                    {candidate.reference}
                  </span>
                  <span className="block truncate text-muted-foreground">
                    {candidate.title}
                  </span>
                </span>
                <span className="shrink-0 text-muted-foreground">
                  {stateLabel(candidate.state)}
                </span>
              </button>
            ))
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
