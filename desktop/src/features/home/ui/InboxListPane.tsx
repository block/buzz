import { ChevronDown, Clock, ExternalLink, MailOpen } from "lucide-react";
import * as React from "react";

import {
  getInboxTypeLabel,
  type InboxFilter,
  type InboxItem,
  type InboxTypeLabel,
} from "@/features/home/lib/inbox";
import { RemindersPanel } from "@/features/reminders/ui/RemindersPanel";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";
import { Markdown } from "@/shared/ui/markdown";
import {
  MENTION_CHIP_BASE_CLASSES,
  MESSAGE_MARKDOWN_CLASS,
} from "@/shared/ui/mentionChip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Switch } from "@/shared/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { VirtualizedList } from "@/shared/ui/VirtualizedList";

const FILTER_OPTIONS: Array<{ label: string; value: InboxFilter }> = [
  { value: "all", label: "All" },
  { value: "mention", label: "Mentions" },
  { value: "thread", label: "Threads" },
  { value: "needs_action", label: "Needs Action" },
  { value: "activity", label: "Activity" },
  { value: "agent_activity", label: "Agents" },
  { value: "reminders", label: "Reminders" },
];

function ActivityLabel({
  isDone,
  isActionRequired,
  label,
}: {
  isDone: boolean;
  isActionRequired: boolean;
  label: InboxTypeLabel;
}) {
  return (
    <div
      className={cn(
        MESSAGE_MARKDOWN_CLASS,
        "mt-0 flex min-w-0 items-center gap-1.5 text-2xs leading-3",
        isActionRequired && !isDone
          ? "font-medium text-amber-600/80 dark:text-amber-300/80"
          : isDone
            ? "font-normal text-muted-foreground/70"
            : "font-medium text-muted-foreground/80",
      )}
    >
      <span className="shrink-0">{label.text}</span>
      {label.channelLabel ? (
        <span
          className={cn(
            MENTION_CHIP_BASE_CLASSES,
            "inbox-channel-chip min-w-0 max-w-full overflow-hidden",
          )}
          data-channel-link=""
        >
          <span className="truncate">#{label.channelLabel}</span>
        </span>
      ) : null}
    </div>
  );
}

type InboxListPaneProps = {
  activeReminderEventIds?: ReadonlySet<string>;
  doneSet: ReadonlySet<string>;
  filter: InboxFilter;
  items: InboxItem[];
  onFilterChange: (filter: InboxFilter) => void;
  onMarkRead: (itemId: string) => void;
  onMarkUnread: (itemId: string) => void;
  onOpenDirect: (item: InboxItem) => void;
  onRemindLater: (item: InboxItem) => void;
  onSelect: (itemId: string) => void;
  onUnreadOnlyChange: (checked: boolean) => void;
  selectedId: string | null;
  showRightDivider?: boolean;
  dueReminderCount: number;
  reminderPubkey?: string;
  unreadOnly: boolean;
};

export function InboxListPane({
  activeReminderEventIds,
  doneSet,
  filter,
  items,
  onFilterChange,
  onMarkRead,
  onMarkUnread,
  onOpenDirect,
  onRemindLater,
  onSelect,
  onUnreadOnlyChange,
  selectedId,
  showRightDivider = false,
  dueReminderCount,
  reminderPubkey,
  unreadOnly,
}: InboxListPaneProps) {
  const activeFilter = FILTER_OPTIONS.find((option) => option.value === filter);
  const isReminders = filter === "reminders";
  const scrollRef = React.useRef<HTMLDivElement>(null);

  const renderItem = (item: InboxItem, index: number) => {
    const isSelected = item.id === selectedId;
    const isDone = doneSet.has(item.id);
    const hasActiveReminder = activeReminderEventIds?.has(item.id) ?? false;
    const hasChannelTarget = Boolean(item.item.channelId);
    const typeLabel = getInboxTypeLabel(item);

    const row = (
      <div
        className="group/inbox-item relative"
        data-testid={`home-inbox-item-${item.id}`}
      >
        <button
          className={cn(
            "relative block w-full border-l px-5 py-2.5 text-left transition-colors after:pointer-events-none after:absolute after:bottom-0 after:left-[3.875rem] after:right-0 after:h-px after:bg-border/45 after:content-['']",
            isSelected
              ? "border-l-transparent bg-muted/30"
              : "border-l-transparent hover:bg-muted/25 active:bg-muted/40",
            index === items.length - 1 && "after:hidden",
          )}
          onClick={() => onSelect(item.id)}
          type="button"
        >
          <div className="flex min-w-0 items-start gap-2.5">
            <div className="relative shrink-0">
              <UserAvatar
                avatarUrl={item.avatarUrl}
                className="h-8 w-8"
                displayName={item.senderLabel}
                size="md"
              />
            </div>

            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 items-start gap-2">
                <p className="min-w-0 flex-1 truncate text-sm font-semibold leading-4 text-foreground">
                  {item.senderLabel}
                </p>
                <span
                  className={cn(
                    "flex shrink-0 items-center gap-1.5 text-xs leading-4 text-muted-foreground/70 transition-opacity group-hover/inbox-item:opacity-0 group-focus-within/inbox-item:opacity-0",
                    isDone ? "font-normal" : "font-medium",
                  )}
                >
                  {!isDone ? (
                    <span
                      aria-hidden="true"
                      className="h-1.5 w-1.5 rounded-full bg-primary"
                    />
                  ) : null}
                  {item.timestampLabel}
                </span>
              </div>
              <ActivityLabel
                isActionRequired={item.isActionRequired}
                isDone={isDone}
                label={typeLabel}
              />
            </div>
          </div>

          <div
            className={cn(
              "mt-1.5 text-sm leading-5 [&_a]:font-medium [&_a]:text-current",
              isDone
                ? "font-normal text-muted-foreground"
                : "font-semibold text-foreground",
            )}
          >
            <Markdown
              className="inbox-preview-markdown text-inherit leading-5"
              content={item.preview}
              interactive={false}
              mentionNames={item.mentionNames}
            />
          </div>
        </button>

        <div className="pointer-events-none absolute right-4 top-2 z-10 origin-top-right -translate-y-0.5 scale-[0.98] opacity-0 transition-[opacity,transform] duration-150 ease-out will-change-[opacity,transform] motion-reduce:translate-y-0 motion-reduce:scale-100 motion-reduce:transition-opacity group-hover/inbox-item:pointer-events-auto group-hover/inbox-item:translate-y-0 group-hover/inbox-item:scale-100 group-hover/inbox-item:opacity-100 group-focus-within/inbox-item:pointer-events-auto group-focus-within/inbox-item:translate-y-0 group-focus-within/inbox-item:scale-100 group-focus-within/inbox-item:opacity-100">
          <div className="overflow-hidden rounded-full border border-border/70 bg-background/95 shadow-xs backdrop-blur-sm supports-[backdrop-filter]:bg-background/85">
            <div className="flex items-center gap-0.5 p-1">
              {isDone ? (
                <InboxRowActionButton
                  label="Mark unread"
                  onClick={() => onMarkUnread(item.id)}
                >
                  <MailOpen className="!h-4 !w-4" />
                </InboxRowActionButton>
              ) : (
                <InboxRowActionButton
                  label="Mark as read"
                  onClick={() => onMarkRead(item.id)}
                >
                  <MailOpen className="!h-4 !w-4" />
                </InboxRowActionButton>
              )}
              <InboxRowActionButton
                disabled={!hasChannelTarget}
                label={hasChannelTarget ? "Open in channel" : "No channel link"}
                onClick={() => onOpenDirect(item)}
              >
                <ExternalLink className="!h-4 !w-4" />
              </InboxRowActionButton>
              <InboxRowActionButton
                active={hasActiveReminder}
                disabled={!hasChannelTarget}
                label={
                  hasChannelTarget
                    ? hasActiveReminder
                      ? "Reminder set"
                      : "Remind me later"
                    : "Cannot remind without a channel"
                }
                onClick={() => onRemindLater(item)}
              >
                <Clock className="!h-4 !w-4" />
              </InboxRowActionButton>
            </div>
          </div>
        </div>
      </div>
    );

    return (
      <ContextMenu>
        <ContextMenuTrigger asChild>{row}</ContextMenuTrigger>
        <ContextMenuContent>
          {isDone ? (
            <ContextMenuItem onClick={() => onMarkUnread(item.id)}>
              <MailOpen className="h-4 w-4" />
              Mark unread
            </ContextMenuItem>
          ) : (
            <ContextMenuItem onClick={() => onMarkRead(item.id)}>
              <MailOpen className="h-4 w-4" />
              Mark as read
            </ContextMenuItem>
          )}
        </ContextMenuContent>
      </ContextMenu>
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
          <div className="flex w-full min-w-0 items-center justify-between gap-3">
            <label
              className={cn(
                "inline-flex shrink-0 items-center gap-2 text-2xs font-medium leading-none text-muted-foreground",
                isReminders && "opacity-50",
              )}
              htmlFor="inbox-unread-only-switch"
            >
              <span>Unread</span>
              <Switch
                checked={unreadOnly}
                className="shadow-none [&>span]:shadow-none"
                data-testid="inbox-unread-only-toggle"
                disabled={isReminders}
                id="inbox-unread-only-switch"
                onCheckedChange={onUnreadOnlyChange}
              />
            </label>
            <div className="ml-auto flex min-w-0 max-w-[var(--home-inbox-list-width)] items-center justify-end">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    className="h-8 shrink-0 px-2.5 text-xs font-medium text-muted-foreground hover:text-foreground"
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
                  {unreadOnly ? "No unread messages" : "No messages found"}
                </p>
                <p className="mt-1 text-sm text-muted-foreground">
                  {unreadOnly
                    ? "Turn off the unread filter to see read messages."
                    : "Switch back to all mail to see more messages."}
                </p>
              </div>
            </div>
          ) : (
            <VirtualizedList
              estimateSize={96}
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

function InboxRowActionButton({
  active = false,
  children,
  disabled = false,
  label,
  onClick,
}: {
  active?: boolean;
  children: React.ReactNode;
  disabled?: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label={label}
          className={cn(
            "flex h-8 w-8 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-40",
            active && "bg-blue-500/10 text-blue-500 hover:text-blue-500",
          )}
          disabled={disabled}
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            if (disabled) {
              return;
            }
            onClick();
          }}
          type="button"
        >
          {children}
        </button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
