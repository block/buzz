import { ChevronDown } from "lucide-react";

import type { InboxFilter } from "@/features/home/lib/inbox";
import { cn } from "@/shared/lib/cn";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

const ACTIVITY_FILTER_OPTIONS: Array<{
  label: string;
  value: InboxFilter;
}> = [
  { value: "all", label: "All" },
  { value: "mention", label: "Mentions" },
  { value: "thread", label: "Threads" },
  { value: "needs_action", label: "Needs action" },
  { value: "agent_activity", label: "Agents" },
  { value: "reminders", label: "Reminders" },
  { value: "drafts", label: "Drafts" },
];

const TRIGGER_CLASS =
  "inline-flex h-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-muted/70 data-[state=open]:text-foreground disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0 relative -ml-2 w-auto gap-1 px-2 text-sm font-medium text-foreground";

type ActivityFilterMenuProps = {
  activeDraftCount: number;
  dueReminderCount: number;
  filter: InboxFilter;
  onFilterChange: (value: InboxFilter) => void;
  reminderCount: number;
};

export function ActivityFilterMenu({
  activeDraftCount,
  dueReminderCount,
  filter,
  onFilterChange,
  reminderCount,
}: ActivityFilterMenuProps) {
  const activeFilter = ACTIVITY_FILTER_OPTIONS.find(
    (option) => option.value === filter,
  );
  const statusLabel =
    dueReminderCount > 0
      ? `${dueReminderCount} due reminder${dueReminderCount === 1 ? "" : "s"}`
      : activeDraftCount > 0
        ? `${activeDraftCount} active draft${activeDraftCount === 1 ? "" : "s"}`
        : null;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={`Filter activity: ${activeFilter?.label ?? "All"}${statusLabel ? `. ${statusLabel}` : ""}`}
          className={cn(TRIGGER_CLASS)}
          data-testid="inbox-filter-trigger"
          type="button"
        >
          <span>{activeFilter?.label ?? "All"}</span>
          <ChevronDown className="text-muted-foreground" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-52">
        <DropdownMenuRadioGroup
          onValueChange={(value) => onFilterChange(value as InboxFilter)}
          value={filter}
        >
          {ACTIVITY_FILTER_OPTIONS.map((option, index) => (
            <div key={option.value}>
              {index === 5 ? (
                <DropdownMenuSeparator className="my-2 bg-border/60" />
              ) : null}
              <DropdownMenuRadioItem value={option.value}>
                <span className="flex flex-1 items-center gap-2">
                  <span>{option.label}</span>
                  <span className="ml-auto flex items-center gap-1.5">
                    {option.value === "reminders" && reminderCount > 0 ? (
                      <span
                        className="inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground"
                        data-testid="inbox-reminder-badge-option"
                      >
                        {reminderCount}
                      </span>
                    ) : option.value === "drafts" && activeDraftCount > 0 ? (
                      <span
                        className="inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground"
                        data-testid="inbox-draft-badge-option"
                      >
                        {activeDraftCount}
                      </span>
                    ) : null}
                  </span>
                </span>
              </DropdownMenuRadioItem>
            </div>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
