import { ChevronDown, Settings, Star } from "lucide-react";
import * as React from "react";

import type {
  ActivityCustomView,
  ActivityViewId,
} from "@/features/home/lib/activityViewPreferences";
import type { InboxFilter } from "@/features/home/lib/inbox";
import { ActivityCustomViewDialog } from "@/features/home/ui/ActivityCustomViewDialog";
import { cn } from "@/shared/lib/cn";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

const FILTER_OPTIONS: Array<{ label: string; value: InboxFilter }> = [
  { value: "all", label: "All" },
  { value: "mention", label: "Mentions" },
  { value: "thread", label: "Threads" },
  { value: "needs_action", label: "Needs Action" },
  { value: "activity", label: "Activity" },
  { value: "agent_activity", label: "Agents" },
  { value: "reminders", label: "Reminders" },
  { value: "drafts", label: "Drafts" },
];

const ACTIVITY_FILTER_OPTIONS: Array<{
  label: string;
  value: ActivityViewId;
}> = [
  { value: "all", label: "All" },
  { value: "mention", label: "Mentions" },
  { value: "thread", label: "Threads" },
  { value: "needs_action", label: "Needs action" },
  { value: "agent_activity", label: "Agents" },
  { value: "reminders", label: "Reminders" },
  { value: "drafts", label: "Drafts" },
  { value: "custom", label: "Custom" },
];

const TRIGGER_CLASS =
  "inline-flex h-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-muted/70 data-[state=open]:text-foreground disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0 relative -ml-2 w-auto gap-1 px-2 text-sm font-medium text-foreground";

type ActivityFilterMenuProps = {
  activityEnabled: boolean;
  activeDraftCount: number;
  customView: ActivityCustomView;
  defaultView: ActivityViewId;
  dueReminderCount: number;
  filter: InboxFilter;
  onCustomViewChange: (value: ActivityCustomView) => void;
  onFilterChange: (value: InboxFilter) => void;
  reminderCount: number;
};

export function ActivityFilterMenu({
  activityEnabled,
  activeDraftCount,
  customView,
  defaultView,
  dueReminderCount,
  filter,
  onCustomViewChange,
  onFilterChange,
  reminderCount,
}: ActivityFilterMenuProps) {
  const [editorOpen, setEditorOpen] = React.useState(false);
  const options = activityEnabled ? ACTIVITY_FILTER_OPTIONS : FILTER_OPTIONS;
  const activeFilter = options.find((option) => option.value === filter);
  const statusLabel =
    dueReminderCount > 0
      ? `${dueReminderCount} due reminder${dueReminderCount === 1 ? "" : "s"}`
      : activeDraftCount > 0
        ? `${activeDraftCount} active draft${activeDraftCount === 1 ? "" : "s"}`
        : null;

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            aria-label={`Filter ${activityEnabled ? "activity" : "inbox"}: ${activeFilter?.label ?? "All"}${statusLabel ? `. ${statusLabel}` : ""}`}
            className={cn(TRIGGER_CLASS)}
            data-testid="inbox-filter-trigger"
            type="button"
          >
            <span>{activeFilter?.label ?? "All"}</span>
            <ChevronDown className="text-muted-foreground" />
            {!activityEnabled &&
            (dueReminderCount > 0 || activeDraftCount > 0) ? (
              <span
                aria-hidden="true"
                className="absolute right-1.5 top-0 h-1.5 w-1.5 rounded-full bg-primary ring-2 ring-background"
                data-testid={
                  dueReminderCount > 0
                    ? "inbox-reminder-badge"
                    : "inbox-draft-badge"
                }
              />
            ) : null}
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-52">
          <DropdownMenuRadioGroup
            onValueChange={(value) => onFilterChange(value as InboxFilter)}
            value={filter}
          >
            {options.map((option) => {
              const radioItem = (
                <DropdownMenuRadioItem
                  className={cn(
                    activityEnabled &&
                      option.value === "custom" &&
                      "min-w-0 flex-1 pr-1",
                  )}
                  key={option.value}
                  value={option.value}
                >
                  <span className="flex flex-1 items-center gap-2">
                    <span>{option.label}</span>
                    <span className="ml-auto flex items-center gap-1.5">
                      {activityEnabled && option.value === defaultView ? (
                        <Star
                          aria-label="Default view"
                          className="h-3.5 w-3.5 fill-current text-muted-foreground"
                        />
                      ) : null}
                      {option.value === "reminders" &&
                      (activityEnabled ? reminderCount : dueReminderCount) >
                        0 ? (
                        <span
                          className="inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground"
                          data-testid="inbox-reminder-badge-option"
                        >
                          {activityEnabled ? reminderCount : dueReminderCount}
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
              );

              if (!activityEnabled || option.value !== "custom") {
                return radioItem;
              }

              return (
                <div className="flex items-center gap-0.5" key={option.value}>
                  {radioItem}
                  <DropdownMenuItem
                    aria-label="Edit Custom view"
                    className="min-h-8 w-12 justify-center p-0"
                    onSelect={() => setEditorOpen(true)}
                    title="Edit Custom view"
                  >
                    <Settings className="h-3.5 w-3.5 text-muted-foreground/70" />
                  </DropdownMenuItem>
                </div>
              );
            })}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
      <ActivityCustomViewDialog
        onOpenChange={setEditorOpen}
        onSave={onCustomViewChange}
        open={editorOpen}
        value={customView}
      />
    </>
  );
}
