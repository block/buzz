import * as React from "react";
import { Check, ChevronDown, Search } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import {
  type PersonaDropdownOption,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./agentConfigOptions";

type PersonaModelComboboxProps = {
  disabled?: boolean;
  id: string;
  onValueChange: (value: string) => void;
  options: readonly PersonaDropdownOption[];
  placeholder: string;
  value: string;
};

export function PersonaModelCombobox({
  disabled,
  id,
  onValueChange,
  options,
  placeholder,
  value,
}: PersonaModelComboboxProps) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [highlightedIndex, setHighlightedIndex] = React.useState(0);

  const selectedOption = options.find((option) => option.value === value);

  const filteredOptions = React.useMemo(() => {
    if (query.trim() === "") return options;
    const lower = query.toLowerCase();
    return options.filter((option) =>
      option.label.toLowerCase().includes(lower),
    );
  }, [options, query]);

  function handleOpenChange(next: boolean) {
    setOpen(next);
    if (!next) {
      setQuery("");
      setHighlightedIndex(0);
    }
  }

  function selectOption(optionValue: string) {
    onValueChange(optionValue);
    handleOpenChange(false);
  }

  // Walk the filtered list from `from` in `direction` (+1 or -1), wrapping
  // around once, and return the first non-disabled index. Returns `from` if
  // every option is disabled so the highlight doesn't vanish unexpectedly.
  function nextEnabledIndex(from: number, direction: 1 | -1): number {
    const len = filteredOptions.length;
    for (let step = 1; step <= len; step++) {
      const candidate = (from + direction * step + len * step) % len;
      if (!filteredOptions[candidate]?.disabled) return candidate;
    }
    return from;
  }

  function handleKeyDown(event: React.KeyboardEvent) {
    switch (event.key) {
      case "ArrowDown": {
        event.preventDefault();
        if (filteredOptions.length > 0) {
          setHighlightedIndex((i) => nextEnabledIndex(i, 1));
        }
        break;
      }
      case "ArrowUp": {
        event.preventDefault();
        if (filteredOptions.length > 0) {
          setHighlightedIndex((i) => nextEnabledIndex(i, -1));
        }
        break;
      }
      case "Enter": {
        event.preventDefault();
        const target = filteredOptions[highlightedIndex];
        if (target && !target.disabled) selectOption(target.value);
        break;
      }
      case "Escape": {
        event.preventDefault();
        handleOpenChange(false);
        break;
      }
    }
  }

  // Reset highlight whenever the filtered list changes so the highlighted
  // row stays within bounds and lands on the first enabled option.
  React.useEffect(() => {
    if (filteredOptions.length === 0) {
      setHighlightedIndex(0);
      return;
    }
    // Walk forward from -1 to land on the first enabled index.
    const len = filteredOptions.length;
    for (let i = 0; i < len; i++) {
      if (!filteredOptions[i]?.disabled) {
        setHighlightedIndex(i);
        return;
      }
    }
    setHighlightedIndex(0);
  }, [filteredOptions]);

  return (
    <div className={PERSONA_FIELD_SHELL_CLASS}>
      <Popover modal={false} onOpenChange={handleOpenChange} open={open}>
        <PopoverTrigger asChild>
          <button
            aria-expanded={open}
            className={cn(
              "flex h-11 w-full items-center justify-between gap-3 px-3 py-2 text-left text-sm leading-6",
              PERSONA_FIELD_CONTROL_CLASS,
              disabled && "cursor-default opacity-60",
            )}
            disabled={disabled}
            id={id}
            role="combobox"
            type="button"
          >
            <span
              className={cn(
                "min-w-0 flex-1 truncate",
                !selectedOption && "text-muted-foreground/55",
              )}
            >
              {selectedOption?.label ?? placeholder}
            </span>
            <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground/60" />
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className="overflow-hidden p-0"
          onOpenAutoFocus={(event) => event.preventDefault()}
          sideOffset={5}
          style={{
            minWidth: "var(--radix-popover-trigger-width)",
            width: "var(--radix-popover-trigger-width)",
          }}
        >
          <div className="group/search flex cursor-text items-center gap-2 border-b border-border/50 px-3 py-2">
            <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground/55 transition-colors duration-150 ease-out group-focus-within/search:text-foreground" />
            <input
              aria-label="Search models"
              autoCapitalize="none"
              autoComplete="off"
              autoCorrect="off"
              className="block min-w-0 flex-1 border-0 bg-transparent p-0 text-sm leading-5 text-muted-foreground/55 shadow-none outline-none placeholder:text-muted-foreground/55 focus:text-foreground focus:placeholder:text-muted-foreground"
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Search models…"
              // Popover supports onOpenAutoFocus; we preventDefault above so
              // Radix doesn't move focus to the first focusable. But we still
              // want the input focused immediately, so use the callback ref.
              ref={(el) => el?.focus()}
              spellCheck={false}
              value={query}
            />
          </div>
          <div
            className="max-h-[min(16rem,var(--radix-popover-content-available-height))] overflow-y-auto overscroll-contain p-1"
            onTouchMoveCapture={(event) => event.stopPropagation()}
            onWheelCapture={(event) => event.stopPropagation()}
          >
            {filteredOptions.length > 0 ? (
              filteredOptions.map((option, index) => (
                <button
                  aria-disabled={option.disabled}
                  className={cn(
                    "relative flex min-h-9 w-full select-none items-center rounded-lg py-2 pl-8 pr-4 text-left text-sm outline-none transition-colors",
                    option.disabled
                      ? "pointer-events-none opacity-50"
                      : "cursor-default hover:bg-muted/50 hover:text-foreground",
                    index === highlightedIndex &&
                      !option.disabled &&
                      "bg-muted/50 text-foreground",
                    option.value === value && "font-medium",
                  )}
                  disabled={option.disabled}
                  key={option.value}
                  onClick={() => selectOption(option.value)}
                  onMouseEnter={() => {
                    if (!option.disabled) setHighlightedIndex(index);
                  }}
                  type="button"
                >
                  <span className="absolute left-2 flex h-3.5 w-3.5 items-center justify-center">
                    <Check
                      className={cn(
                        "h-3.5 w-3.5",
                        option.value === value ? "opacity-100" : "opacity-0",
                      )}
                    />
                  </span>
                  <span className="truncate">{option.label}</span>
                </button>
              ))
            ) : (
              <p className="px-3 py-6 text-center text-sm text-muted-foreground/55">
                No models match
              </p>
            )}
          </div>
        </PopoverContent>
      </Popover>
    </div>
  );
}
