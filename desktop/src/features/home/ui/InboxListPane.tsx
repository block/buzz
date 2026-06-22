import {
  Clock,
  Ellipsis,
  ExternalLink,
  ListFilter,
  MailOpen,
} from "lucide-react";
import * as React from "react";

import {
  getInboxTypeLabel,
  type InboxFilter,
  type InboxItem,
  type InboxTypeLabel,
} from "@/features/home/lib/inbox";
import { RemindersPanel } from "@/features/reminders/ui/RemindersPanel";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { cn } from "@/shared/lib/cn";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
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
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Separator } from "@/shared/ui/separator";
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

const INBOX_HEADER_ICON_BUTTON_CLASS =
  "inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring data-[state=open]:bg-muted/70 data-[state=open]:text-foreground disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0";
const INBOX_PANE_RIGHT_DIVIDER_CLASS =
  "after:pointer-events-none after:absolute after:inset-y-0 after:right-0 after:z-40 after:w-px after:bg-border/35 after:content-['']";

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
        "mt-0 flex min-w-0 items-center gap-1.5 text-2xs leading-3 group-hover/inbox-item:pr-[6.75rem] group-focus-within/inbox-item:pr-[6.75rem]",
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
  const unreadVisibleItemCount = React.useMemo(
    () =>
      items.reduce((count, item) => count + (doneSet.has(item.id) ? 0 : 1), 0),
    [doneSet, items],
  );
  const handleMarkAllRead = React.useCallback(() => {
    for (const item of items) {
      if (!doneSet.has(item.id)) {
        onMarkRead(item.id);
      }
    }
  }, [doneSet, items, onMarkRead]);

  const renderItem = (item: InboxItem, index: number) => {
    const isSelected = item.id === selectedId;
    const isDone = doneSet.has(item.id);
    const hasActiveReminder = activeReminderEventIds?.has(item.id) ?? false;
    const hasChannelTarget = Boolean(item.item.channelId);
    const typeLabel = getInboxTypeLabel(item);
    const rowHighlightColor = isSelected
      ? "color-mix(in srgb, hsl(var(--background)) 70%, hsl(var(--muted)) 30%)"
      : "color-mix(in srgb, hsl(var(--background)) 75%, hsl(var(--muted)) 25%)";

    const row = (
      <div
        className="group/inbox-item relative"
        data-testid={`home-inbox-item-${item.id}`}
        style={
          {
            "--inbox-row-highlight-bg": rowHighlightColor,
          } as React.CSSProperties
        }
      >
        <button
          className={cn(
            "relative block w-full border-l px-3 py-4 text-left transition-colors after:pointer-events-none after:absolute after:bottom-0 after:left-[3.375rem] after:right-0 after:h-px after:bg-border/45 after:content-['']",
            isSelected
              ? "border-l-transparent bg-[var(--inbox-row-highlight-bg)]"
              : "border-l-transparent group-hover/inbox-item:bg-[var(--inbox-row-highlight-bg)] group-focus-within/inbox-item:bg-[var(--inbox-row-highlight-bg)] active:bg-muted/40",
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
            </div>
          </div>
        </button>

        <div className="pointer-events-none absolute right-2 top-2 z-10 flex items-center gap-0.5 rounded-full bg-[var(--inbox-row-highlight-bg)] p-1 opacity-0 transition-opacity duration-150 ease-out group-hover/inbox-item:pointer-events-auto group-hover/inbox-item:opacity-100 group-focus-within/inbox-item:pointer-events-auto group-focus-within/inbox-item:opacity-100">
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
          <ContextMenuSeparator />
          <ContextMenuItem
            disabled={!hasChannelTarget}
            onClick={() => {
              if (hasChannelTarget) {
                onOpenDirect(item);
              }
            }}
          >
            <ExternalLink className="h-4 w-4" />
            {hasChannelTarget ? "Open in channel" : "No channel link"}
          </ContextMenuItem>
          <ContextMenuItem
            disabled={!hasChannelTarget}
            onClick={() => {
              if (hasChannelTarget) {
                onRemindLater(item);
              }
            }}
          >
            <Clock className="h-4 w-4" />
            {hasActiveReminder ? "Reminder set" : "Remind me later"}
          </ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>
    );
  };

  return (
    <section
      className={cn(
        "relative flex min-h-0 min-w-0 flex-col overflow-hidden bg-background/60",
        showRightDivider && INBOX_PANE_RIGHT_DIVIDER_CLASS,
      )}
    >
      <TopChromeInsetHeader flush>
        <div className="px-3 py-1">
          <div className="flex w-full min-w-0 items-center justify-between gap-3">
            <Popover>
              <PopoverTrigger asChild>
                <button
                  aria-label="Inbox options"
                  className={INBOX_HEADER_ICON_BUTTON_CLASS}
                  data-testid="inbox-options-trigger"
                  type="button"
                >
                  <Ellipsis className="h-4 w-4" />
                </button>
              </PopoverTrigger>
              <PopoverContent align="start" className="w-60 p-2">
                <div
                  className={cn(
                    "flex min-h-9 items-center justify-between gap-3 rounded-lg px-2 py-1.5",
                    isReminders && "opacity-50",
                  )}
                >
                  <label
                    className="text-sm font-medium text-foreground"
                    htmlFor="inbox-unread-only-switch"
                  >
                    Show unread
                  </label>
                  <Switch
                    checked={unreadOnly}
                    className="shadow-none [&>span]:shadow-none"
                    data-testid="inbox-unread-only-toggle"
                    disabled={isReminders}
                    id="inbox-unread-only-switch"
                    onCheckedChange={onUnreadOnlyChange}
                  />
                </div>
                <Separator className="my-1 bg-muted" />
                <button
                  className="flex min-h-9 w-full items-center rounded-lg px-2 py-2 text-left text-sm transition-colors hover:bg-muted/50 disabled:pointer-events-none disabled:opacity-50"
                  disabled={unreadVisibleItemCount === 0}
                  onClick={handleMarkAllRead}
                  type="button"
                >
                  <span>Mark all as read</span>
                  {unreadVisibleItemCount > 0 ? (
                    <span className="ml-auto text-xs text-muted-foreground">
                      {unreadVisibleItemCount}
                    </span>
                  ) : null}
                </button>
              </PopoverContent>
            </Popover>
            <div className="ml-auto flex shrink-0 items-center justify-end">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button
                    aria-label={`Filter inbox: ${activeFilter?.label ?? "All"}`}
                    className={cn(INBOX_HEADER_ICON_BUTTON_CLASS, "relative")}
                    data-testid="inbox-filter-trigger"
                    type="button"
                  >
                    <ListFilter className="h-4 w-4" />
                    {dueReminderCount > 0 ? (
                      <span
                        className="absolute -right-1 -top-1 inline-flex h-4 min-w-4 items-center justify-center rounded-full border border-background bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground"
                        data-testid="inbox-reminder-badge"
                      >
                        {dueReminderCount}
                      </span>
                    ) : null}
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="w-48">
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
