import { ChevronDown } from "lucide-react";

import { useTranslation } from "@/i18n";
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

const INBOX_FILTER_OPTIONS: Array<{ value: InboxFilter }> = [
  { value: "all" },
  { value: "project" },
  { value: "mention" },
  { value: "thread" },
  { value: "needs_action" },
  { value: "agent_activity" },
  { value: "reminders" },
  { value: "drafts" },
];

const TRIGGER_CLASS =
  "inline-flex h-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-muted/70 data-[state=open]:text-foreground disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0 relative -ml-2 w-auto gap-1 px-2 text-sm font-medium text-foreground";

type InboxFilterMenuProps = {
  activeDraftCount: number;
  dueReminderCount: number;
  filter: InboxFilter;
  onFilterChange: (value: InboxFilter) => void;
  reminderCount: number;
};

export function InboxFilterMenu({
  activeDraftCount,
  dueReminderCount,
  filter,
  onFilterChange,
  reminderCount,
}: InboxFilterMenuProps) {
  const { t } = useTranslation();
  const filterLabels: Record<InboxFilter, string> = {
    all: t("home.filter.all"),
    project: t("sidebar.nav.projects"),
    mention: t("home.filter.mentions"),
    thread: t("home.filter.threads"),
    needs_action: t("home.filter.needs-action"),
    agent_activity: t("sidebar.nav.agents"),
    reminders: t("home.filter.reminders"),
    drafts: t("messages.drafts.heading"),
  };
  const activeLabel = filterLabels[filter] ?? filterLabels.all;
  const statusLabel =
    dueReminderCount > 0
      ? t("home.filter.due-reminders", { count: dueReminderCount })
      : activeDraftCount > 0
        ? t("home.filter.active-drafts", { count: activeDraftCount })
        : null;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={
            statusLabel
              ? t("home.filter.trigger-aria-status", {
                  filter: activeLabel,
                  status: statusLabel,
                })
              : t("home.filter.trigger-aria", { filter: activeLabel })
          }
          className={cn(TRIGGER_CLASS)}
          data-testid="inbox-filter-trigger"
          type="button"
        >
          <span>{activeLabel}</span>
          <ChevronDown className="text-muted-foreground" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-52">
        <DropdownMenuRadioGroup
          onValueChange={(value) => onFilterChange(value as InboxFilter)}
          value={filter}
        >
          {INBOX_FILTER_OPTIONS.map((option) => (
            <div key={option.value}>
              {option.value === "reminders" ? (
                <DropdownMenuSeparator className="my-2 bg-border/60" />
              ) : null}
              <DropdownMenuRadioItem value={option.value}>
                <span className="flex flex-1 items-center gap-2">
                  <span>{filterLabels[option.value]}</span>
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
