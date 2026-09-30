import { Search, X } from "lucide-react";
import type * as React from "react";

import {
  AGENT_LIBRARY_STATUS_FILTERS,
  type AgentLibraryCounts,
  type AgentLibraryStatusFilter,
  isPlainEscapeKey,
} from "@/features/agents/lib/agentLibraryFilter";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

const STATUS_CHIP_CLASS =
  "h-7 shrink-0 gap-1.5 rounded-full bg-muted/30 px-3 text-xs font-medium leading-5 tracking-tight text-muted-foreground shadow-none transition-colors hover:bg-muted/55 hover:text-foreground";
const STATUS_CHIP_SELECTED_CLASS = "bg-muted text-foreground";

/**
 * Search box plus lifecycle chips for the Agents library. The parent owns
 * the values; this component only renders and reports edits.
 */
export function AgentLibraryToolbar({
  counts,
  inputRef,
  onQueryChange,
  onStatusChange,
  query,
  status,
}: {
  counts: AgentLibraryCounts;
  /** Owned by the parent so its empty-state Clear can return focus here. */
  inputRef: React.RefObject<HTMLInputElement | null>;
  onQueryChange: (query: string) => void;
  onStatusChange: (status: AgentLibraryStatusFilter) => void;
  query: string;
  status: AgentLibraryStatusFilter;
}) {
  function clearQuery() {
    onQueryChange("");
    inputRef.current?.focus();
  }

  return (
    <div
      className="flex flex-wrap items-center gap-2"
      data-testid="agents-library-toolbar"
    >
      <div className="relative min-w-56 flex-1 [@container(min-width:40rem)]:max-w-sm">
        <Search className="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          aria-label="Search agents"
          className="h-8 pl-9 pr-9 text-sm [&::-webkit-search-cancel-button]:hidden"
          data-testid="agents-library-search-input"
          onChange={(event) => onQueryChange(event.target.value)}
          onKeyDown={(event) => {
            if (!isPlainEscapeKey(event) || query.length === 0) return;
            event.preventDefault();
            clearQuery();
          }}
          placeholder="Search names, descriptions, instructions"
          ref={inputRef}
          type="search"
          value={query}
        />
        {query.length > 0 ? (
          <Button
            aria-label="Clear agent search"
            className="absolute top-1/2 right-1 -translate-y-1/2 text-muted-foreground hover:text-foreground"
            data-testid="agents-library-search-clear"
            onClick={clearQuery}
            size="icon-xs"
            type="button"
            variant="ghost"
          >
            <X />
          </Button>
        ) : null}
      </div>
      <fieldset className="flex flex-wrap items-center gap-1.5">
        <legend className="sr-only">Filter agents by status</legend>
        {AGENT_LIBRARY_STATUS_FILTERS.map((option) => {
          const selected = status === option.value;
          return (
            <Button
              aria-label={`${option.label}, ${counts[option.value]}`}
              aria-pressed={selected}
              className={cn(
                STATUS_CHIP_CLASS,
                selected && STATUS_CHIP_SELECTED_CLASS,
              )}
              data-testid={`agents-library-status-${option.value}`}
              key={option.value}
              onClick={() => onStatusChange(option.value)}
              type="button"
              variant="ghost"
            >
              {option.label}
              <span
                className="tabular-nums text-muted-foreground/80"
                data-testid={`agents-library-status-count-${option.value}`}
              >
                {counts[option.value]}
              </span>
            </Button>
          );
        })}
      </fieldset>
    </div>
  );
}
