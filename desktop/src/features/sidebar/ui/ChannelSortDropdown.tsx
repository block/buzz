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
 * Section-header dropdown for the sidebar-wide channel sort preference.
 * One preference, applied inside every grouping (Starred, custom sections,
 * Channels, Forums, DMs) without changing grouping boundaries.
 */
export function ChannelSortDropdown({
  sortMode,
  onSortModeChange,
}: {
  sortMode: ChannelSortMode;
  onSortModeChange: (mode: ChannelSortMode) => void;
}) {
  const activeLabel =
    SORT_OPTIONS.find((option) => option.value === sortMode)?.label ?? "A–Z";

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={`Sort channels: ${activeLabel}`}
          className={cn(
            SECTION_ICON_BUTTON_CLASS,
            SECTION_ACTION_VISIBILITY_CLASS,
          )}
          data-testid="channel-sort-trigger"
          title={`Sort channels: ${activeLabel}`}
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
