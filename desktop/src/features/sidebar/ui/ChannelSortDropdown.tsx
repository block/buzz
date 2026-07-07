import { ArrowUpDown } from "lucide-react";

import type { ChannelSortMode } from "@/features/sidebar/lib/channelSortPreference";
import {
  SECTION_ACTION_VISIBILITY_CLASS,
  SECTION_ICON_BUTTON_CLASS,
} from "@/features/sidebar/ui/sidebarSectionStyles";
import { cn } from "@/shared/lib/cn";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

const SORT_OPTIONS: { value: ChannelSortMode; label: string }[] = [
  { value: "recent", label: "Recent" },
  { value: "alpha", label: "A–Z" },
];

/**
 * Section-header dropdown for a single sidebar grouping's sort preference.
 * Every grouping (Starred, each custom section, Channels, Forums, DMs)
 * carries its own control and saved mode; grouping boundaries are untouched.
 */
export function ChannelSortDropdown({
  groupLabel,
  sortMode,
  onSortModeChange,
  testId,
  visibilityClassName = SECTION_ACTION_VISIBILITY_CLASS,
}: {
  groupLabel: string;
  sortMode: ChannelSortMode;
  onSortModeChange: (mode: ChannelSortMode) => void;
  testId?: string;
  visibilityClassName?: string;
}) {
  const activeLabel =
    SORT_OPTIONS.find((option) => option.value === sortMode)?.label ?? "A–Z";
  const ariaLabel = `Sort ${groupLabel}: ${activeLabel}`;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={ariaLabel}
          className={cn(SECTION_ICON_BUTTON_CLASS, visibilityClassName)}
          data-testid={testId ?? "channel-sort-trigger"}
          onClick={(e) => e.stopPropagation()}
          title={ariaLabel}
          type="button"
        >
          <ArrowUpDown className="h-4 w-4" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-40">
        <DropdownMenuRadioGroup
          onValueChange={(value) => onSortModeChange(value as ChannelSortMode)}
          value={sortMode}
        >
          {SORT_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              {option.label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
