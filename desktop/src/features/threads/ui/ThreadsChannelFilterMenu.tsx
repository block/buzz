import { ChevronDown } from "lucide-react";

import type { ThreadChannelOption } from "@/features/threads/lib/threadChannelFilter";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

const ALL_CHANNELS = "__all__";

const TRIGGER_CLASS =
  "inline-flex h-8 min-w-0 max-w-64 shrink items-center justify-center gap-1 rounded-lg px-2 text-sm font-medium text-foreground transition-colors hover:bg-muted/70 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-muted/70 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0";

/** Narrows the Threads view to one channel; lists only channels with named threads. */
export function ThreadsChannelFilterMenu({
  onChange,
  options,
  selected,
  totalCount,
}: {
  onChange: (channelId: string | null) => void;
  options: readonly ThreadChannelOption[];
  selected: string | null;
  totalCount: number;
}) {
  const active = options.find((option) => option.channelId === selected);
  const label = active?.label ?? "All channels";

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={`Filter threads by channel: ${label}`}
          className={TRIGGER_CLASS}
          data-testid="threads-channel-filter"
          type="button"
        >
          <span className="min-w-0 truncate">{label}</span>
          <ChevronDown className="text-muted-foreground" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="max-h-96 w-64 overflow-y-auto"
      >
        <DropdownMenuRadioGroup
          onValueChange={(value) =>
            onChange(value === ALL_CHANNELS ? null : value)
          }
          value={selected ?? ALL_CHANNELS}
        >
          <FilterOption
            count={totalCount}
            label="All channels"
            value={ALL_CHANNELS}
          />
          {options.length > 0 ? (
            <DropdownMenuSeparator className="my-1 bg-border/60" />
          ) : null}
          {options.map((option) => (
            <FilterOption
              count={option.count}
              key={option.channelId}
              label={option.label}
              value={option.channelId}
            />
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function FilterOption({
  count,
  label,
  value,
}: {
  count: number;
  label: string;
  value: string;
}) {
  return (
    <DropdownMenuRadioItem
      data-testid="threads-channel-filter-option"
      value={value}
    >
      <span className="flex min-w-0 flex-1 items-center gap-2">
        <span className="min-w-0 flex-1 truncate">{label}</span>
        <span className="shrink-0 text-xs text-muted-foreground">{count}</span>
      </span>
    </DropdownMenuRadioItem>
  );
}
