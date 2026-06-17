import { ChevronDown } from "lucide-react";
import * as React from "react";

import {
  formatInboxTypeLabel,
  type InboxFilter,
  type InboxItem,
} from "@/features/home/lib/inbox";
import { RemindersPanel } from "@/features/reminders/ui/RemindersPanel";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { VirtualizedList } from "@/shared/ui/VirtualizedList";

const FILTER_OPTIONS: Array<{ label: string; value: InboxFilter }> = [
  { value: "all", label: "All" },
  { value: "mention", label: "Mentions" },
  { value: "needs_action", label: "Needs Action" },
  { value: "activity", label: "Activity" },
  { value: "agent_activity", label: "Agents" },
  { value: "reminders", label: "Reminders" },
];

type InboxListPaneProps = {
  doneSet: ReadonlySet<string>;
  filter: InboxFilter;
  items: InboxItem[];
  onFilterChange: (filter: InboxFilter) => void;
  onSelect: (itemId: string) => void;
  selectedId: string | null;
  showRightDivider?: boolean;
  dueReminderCount: number;
  reminderPubkey?: string;
};

export function InboxListPane({
  doneSet,
  filter,
  items,
  onFilterChange,
  onSelect,
  selectedId,
  showRightDivider = false,
  dueReminderCount,
  reminderPubkey,
}: InboxListPaneProps) {
  const activeFilter = FILTER_OPTIONS.find((option) => option.value === filter);
  const isReminders = filter === "reminders";
  const scrollRef = React.useRef<HTMLDivElement>(null);

  const renderItem = (item: InboxItem) => {
    const isSelected = item.id === selectedId;
    const isDone = doneSet.has(item.id);
    const typeLabel = formatInboxTypeLabel(item);

    return (
      <button
        className={cn(
          "flex w-full items-start gap-2.5 border-l px-5 py-2 text-left transition-colors",
          isSelected
            ? "border-l-transparent bg-muted/30"
            : "border-l-transparent hover:bg-muted/25 active:bg-muted/40",
        )}
        data-testid={`home-inbox-item-${item.id}`}
        onClick={() => onSelect(item.id)}
        type="button"
      >
        <div className="relative">
          <UserAvatar
            avatarUrl={item.avatarUrl}
            className="h-8 w-8"
            displayName={item.senderLabel}
            size="md"
          />
          {!isDone ? (
            <span className="absolute -right-1 -top-1 h-2.5 w-2.5 rounded-full border-2 border-background bg-primary" />
          ) : null}
        </div>

        <div className="min-w-0 flex-1">
          <div className="flex items-start gap-2">
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <p className="truncate text-sm font-semibold text-foreground">
                  {item.senderLabel}
                </p>
                {item.isActionRequired ? (
                  <span className="inline-flex shrink-0 items-center text-2xs font-semibold uppercase tracking-[0.14em] text-amber-600 dark:text-amber-300">
                    Needs action
                  </span>
                ) : null}
              </div>
            </div>
            <span
              className={cn(
                "shrink-0 text-xs text-muted-foreground",
                isDone ? "font-normal" : "font-semibold",
              )}
            >
              {item.timestampLabel}
            </span>
          </div>

          <div
            className={cn(
              "mt-0.5 line-clamp-2 text-sm leading-5 **:inline [&_a]:font-medium [&_a]:text-current [&_br]:hidden [&_p]:inline",
              isDone
                ? "font-normal text-muted-foreground"
                : "font-semibold text-foreground",
            )}
          >
            <Markdown
              className="inline max-w-full text-inherit"
              content={item.preview}
              interactive={false}
              mentionNames={item.mentionNames}
            />
          </div>

          <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span
              className={cn(
                "text-2xs text-muted-foreground",
                isDone ? "font-normal" : "font-semibold",
              )}
            >
              {typeLabel}
            </span>
          </div>
        </div>
      </button>
    );
  };

  return (
    <section
      className={cn(
        "relative flex min-h-0 min-w-0 flex-col overflow-hidden bg-background/60",
        showRightDivider && topChromeInset.verticalDivider,
      )}
    >
      <TopChromeInsetHeader>
        <div className="px-5 py-1">
          {/* Cap to the list-column width so the right-aligned dropdown stays
              put when the pane goes full-width in reminders mode. */}
          <div className="flex min-w-0 max-w-[var(--home-inbox-list-width)] items-center justify-end gap-3">
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  className="inline-flex shrink-0 items-center gap-1.5 rounded-full border-border/70 bg-background/70 px-2.5 text-2xs font-medium leading-none text-muted-foreground shadow-xs backdrop-blur-sm hover:bg-muted/60 hover:text-foreground"
                  data-testid="inbox-filter-trigger"
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  <span>{activeFilter?.label ?? "All"}</span>
                  {dueReminderCount > 0 ? (
                    <span
                      className="inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground"
                      data-testid="inbox-reminder-badge"
                    >
                      {dueReminderCount}
                    </span>
                  ) : null}
                  <ChevronDown className="h-4 w-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="min-w-40">
                <DropdownMenuRadioGroup
                  onValueChange={(value) =>
                    onFilterChange(value as InboxFilter)
                  }
                  value={filter}
                >
                  {FILTER_OPTIONS.map((option) => (
                    <DropdownMenuRadioItem
                      key={option.value}
                      value={option.value}
                    >
                      <span className="flex flex-1 items-center justify-between gap-2">
                        {option.label}
                        {option.value === "reminders" &&
                        dueReminderCount > 0 ? (
                          <span
                            className="inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground"
                            data-testid="inbox-reminder-badge-option"
                          >
                            {dueReminderCount}
                          </span>
                        ) : null}
                      </span>
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>
      </TopChromeInsetHeader>

      {isReminders ? (
        <div
          className="flex min-h-0 flex-1 flex-col overflow-hidden"
          data-testid="home-inbox-reminders"
        >
          {reminderPubkey ? (
            <RemindersPanel includeDone pubkey={reminderPubkey} />
          ) : null}
        </div>
      ) : (
        <div
          className="min-h-0 flex-1 overflow-y-auto overscroll-contain"
          data-testid="home-inbox-list"
          ref={scrollRef}
        >
          {items.length === 0 ? (
            <div className="flex h-full min-h-64 items-center justify-center px-6 text-center">
              <div>
                <p className="text-sm font-medium text-foreground">
                  No messages found
                </p>
                <p className="mt-1 text-sm text-muted-foreground">
                  Switch back to all mail to see more messages.
                </p>
              </div>
            </div>
          ) : (
            <VirtualizedList
              estimateSize={76}
              getItemKey={(item) => item.id}
              items={items}
              renderItem={renderItem}
              scrollRef={scrollRef}
            />
          )}
        </div>
      )}
    </section>
  );
}
