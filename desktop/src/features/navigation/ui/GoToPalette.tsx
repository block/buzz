import * as React from "react";

import type { GoToPaletteState } from "@/features/navigation/useGoToPalette";
import { cn } from "@/shared/lib/cn";
import { isMacPlatform } from "@/shared/lib/platform";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "@/shared/ui/dialog";

type GoToPaletteProps = {
  state: GoToPaletteState;
};

function mnemonicHint(mnemonic: string): string {
  return isMacPlatform() ? `\u2318${mnemonic}` : `Ctrl+${mnemonic}`;
}

/**
 * The ⌘G "Go to" palette surface. Purely presentational — all keyboard
 * behavior (leader, accelerators, arrows/Enter) is owned by `useGoToPalette`
 * via a capture-phase listener; this component only renders and wires the
 * filter input, rows, and click/hover.
 */
export function GoToPalette({ state }: GoToPaletteProps) {
  const {
    open,
    query,
    results,
    selectedIndex,
    onOpenChange,
    onQueryChange,
    onHoverIndex,
    onSelect,
  } = state;
  const inputRef = React.useRef<HTMLInputElement>(null);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="mt-[18vh] max-w-md self-start gap-0 overflow-hidden rounded-2xl p-0 shadow-2xl"
        data-testid="go-to-palette"
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          inputRef.current?.focus();
        }}
        showCloseButton={false}
      >
        <DialogTitle className="sr-only">Go to</DialogTitle>
        <DialogDescription className="sr-only">
          Jump to a section of the app. Type to filter, press a number to jump
          by position, or use the shortcut shown on each row.
        </DialogDescription>
        <div className="flex items-center border-b border-border/60 px-4 py-3">
          <input
            className="w-full bg-transparent text-base text-foreground placeholder:text-muted-foreground/60 focus:outline-hidden"
            data-testid="go-to-input"
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder="Go to&hellip;"
            ref={inputRef}
            type="text"
            value={query}
          />
        </div>
        {results.length === 0 ? (
          <p className="px-4 py-5 text-sm text-muted-foreground">
            No areas match <span className="font-semibold">{query}</span>.
          </p>
        ) : (
          <div aria-label="App areas" className="p-1.5" role="listbox">
            {results.map((result, index) => {
              const Icon = result.destination.icon;
              const isSelected = index === selectedIndex;
              return (
                <button
                  aria-selected={isSelected}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-lg px-2.5 py-2.5 text-left transition-colors",
                    isSelected
                      ? "bg-muted/45 text-foreground"
                      : "hover:bg-muted/35",
                  )}
                  data-testid={`go-to-item-${result.destination.id}`}
                  key={result.destination.id}
                  onClick={() => onSelect(result.destination.id)}
                  onMouseEnter={() => onHoverIndex(index)}
                  role="option"
                  type="button"
                >
                  <kbd className="flex h-5 w-5 shrink-0 items-center justify-center rounded border border-border/60 bg-muted/50 font-mono text-2xs text-muted-foreground">
                    {result.position}
                  </kbd>
                  <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-background/70 text-muted-foreground">
                    <Icon className="h-4 w-4" />
                  </span>
                  <span className="min-w-0 flex-1 truncate text-sm font-semibold">
                    {result.destination.label}
                  </span>
                  <kbd className="shrink-0 rounded border border-border/60 bg-muted/50 px-1.5 py-0.5 font-mono text-2xs text-muted-foreground">
                    {mnemonicHint(result.destination.mnemonic)}
                  </kbd>
                </button>
              );
            })}
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
