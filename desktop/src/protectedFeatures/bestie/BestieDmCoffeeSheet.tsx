import { ChevronDown, Coffee } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { BESTIE_COFFEE_LIVE_LABEL } from "./bestieCoffeeLive";
import { removeBestieCoffeeEntryForScope, useBestieCoffee } from "./bestieCoffeeStore";
import type {
  BestieCoffeeEntry,
  BestieCoffeeScope,
} from "./bestieCoffeeTypes";

function formatCoffeeDate(ranAt: number): string {
  return new Date(ranAt * 1000).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

function CoffeeRow({
  entry,
  expanded,
  onToggle,
  onRemove,
}: {
  entry: BestieCoffeeEntry;
  expanded: boolean;
  onRemove: () => void;
  onToggle: () => void;
}) {
  return (
    <div
      className="rounded-md border border-border/60 bg-muted/25"
      data-testid={`bestie-coffee-item-${entry.id}`}
    >
      <button
        aria-expanded={expanded}
        className="flex w-full items-start gap-2 px-2 py-1.5 text-left"
        data-testid={`bestie-coffee-toggle-${entry.id}`}
        onClick={onToggle}
        type="button"
      >
        <ChevronDown
          className={cn(
            "mt-0.5 size-3.5 shrink-0 text-muted-foreground transition-transform",
            !expanded && "-rotate-90",
          )}
        />
        <div className="min-w-0 flex-1">
          <p className="text-2xs text-muted-foreground">
            {formatCoffeeDate(entry.ranAt)}
            {entry.source === "brew" ? " · Brew" : ""}
          </p>
          <p className="text-sm leading-snug">{entry.brief}</p>
        </div>
      </button>
      {expanded ? (
        <div
          className="border-t border-border/50 px-2 py-2"
          data-testid={`bestie-coffee-expand-${entry.id}`}
        >
          <pre className="overflow-x-auto whitespace-pre font-mono text-sm leading-normal text-foreground/90">
            {entry.fullOutput}
          </pre>
          <div className="mt-2 flex justify-end">
            <Button
              aria-label="Remove coffee entry"
              className="h-7 px-2 text-xs"
              data-testid={`bestie-coffee-remove-${entry.id}`}
              onClick={onRemove}
              size="sm"
              type="button"
              variant="ghost"
            >
              Remove
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

/**
 * Coffee category sheet — Brew button + expandable briefing history.
 * Scheduled runs and Brew share the same invoke path (/hula-coffee turn).
 * Brewing UI follows the real ACP in-flight coffee turn (🤔…), not eyes alone.
 */
export function BestieDmCoffeeSheet({
  brewDisabled,
  coffeeLive,
  onBrew,
  scope,
}: {
  /** Disable Brew while /hula-coffee is actually running (ACP live / grace). */
  brewDisabled: boolean;
  /** Show thinking-face live indicator on the sheet. */
  coffeeLive: boolean;
  onBrew: () => void;
  scope: BestieCoffeeScope;
}) {
  const state = useBestieCoffee(scope);
  const [expandedId, setExpandedId] = React.useState<string | null>(null);

  return (
    <div
      className="flex flex-col gap-3 py-1"
      data-testid="bestie-dm-coffee-sheet"
    >
      <div className="flex items-center gap-2">
        <Button
          className="h-8 gap-1.5"
          data-testid="bestie-coffee-brew"
          disabled={brewDisabled}
          onClick={() => {
            if (brewDisabled) return;
            onBrew();
          }}
          size="sm"
          type="button"
          variant="secondary"
        >
          <Coffee className="size-3.5" />
          {brewDisabled ? "Brewing…" : "Brew"}
        </Button>
        {coffeeLive ? (
          <span
            aria-live="polite"
            className="text-xs text-muted-foreground"
            data-testid="bestie-coffee-brewing"
          >
            <span aria-hidden="true">{BESTIE_COFFEE_LIVE_LABEL}</span>{" "}
            Running /hula-coffee…
          </span>
        ) : (
          <span className="text-2xs text-muted-foreground">
            Daily at{" "}
            {String(state.prefs.hour).padStart(2, "0")}:
            {String(state.prefs.minute).padStart(2, "0")} local when online
          </span>
        )}
      </div>
      <div className="space-y-1.5">
        {state.entries.length === 0 ? (
          <p className="px-0.5 text-xs text-muted-foreground">
            No coffee briefings yet. Brew now, or wait for the morning run.
          </p>
        ) : (
          state.entries.map((entry) => (
            <CoffeeRow
              key={entry.id}
              entry={entry}
              expanded={expandedId === entry.id}
              onRemove={() => removeBestieCoffeeEntryForScope(scope, entry.id)}
              onToggle={() =>
                setExpandedId((current) =>
                  current === entry.id ? null : entry.id,
                )
              }
            />
          ))
        )}
      </div>
    </div>
  );
}
