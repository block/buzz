import {
  Bell,
  ChevronDown,
  Clock,
  Ellipsis,
  ExternalLink,
  FileText,
  MailOpen,
} from "lucide-react";
import * as React from "react";

import {
  getInboxTypeLabel,
  type InboxFilter,
  type InboxItem,
  type InboxTypeLabel,
} from "@/features/home/lib/inbox";
import { buildActivityListRows } from "@/features/home/lib/activityListRows";
import {
  DraftsPanel,
  getDraftPreview,
  type DraftViewItem,
} from "@/features/messages/ui/DraftsPanel";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import type { Reminder } from "@/features/reminders/lib/reminderTypes";
import {
  RemindersPanel,
  useReminderSources,
} from "@/features/reminders/ui/RemindersPanel";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
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
  { value: "drafts", label: "Drafts" },
];

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

const ACTIVITY_EMPTY_STATE_TITLES: Record<InboxFilter, string> = {
  all: "No activity yet",
  mention: "No mentions found",
  thread: "No threads found",
  needs_action: "Nothing needs action",
  activity: "No activity found",
  agent_activity: "No agent updates found",
  reminders: "No reminders",
  drafts: "No drafts",
};

const ACTIVITY_UNREAD_EMPTY_STATE_TITLES: Record<InboxFilter, string> = {
  all: "No unread activity",
  mention: "No unread mentions",
  thread: "No unread threads",
  needs_action: "No unread items needing action",
  activity: "No unread activity",
  agent_activity: "No unread agent updates",
  reminders: "No unread reminders",
  drafts: "No unread drafts",
};

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

function formatReminderStatus(notBefore: number | undefined) {
  if (notBefore === undefined) return "Pending";
  const secondsUntil = notBefore - Math.floor(Date.now() / 1_000);
  if (secondsUntil <= 0) return "Reminder due";
  if (secondsUntil < 60) return "Reminder in less than a minute";
  if (secondsUntil < 3_600) {
    return `Reminder in ${Math.floor(secondsUntil / 60)}m`;
  }
  if (secondsUntil < 86_400) {
    return `Reminder in ${Math.floor(secondsUntil / 3_600)}h`;
  }
  return `Reminder in ${Math.floor(secondsUntil / 86_400)}d`;
}

function PersonalItemRow({
  id,
  kind,
  location,
  onClick,
  preview,
  status,
}: {
  id: string;
  kind: "drafts" | "reminders";
  location: InboxTypeLabel | null;
  onClick: () => void;
  preview: string;
  status: string;
}) {
  const isDraft = kind === "drafts";
  const Icon = isDraft ? FileText : Bell;

  return (
    <button
      className="flex w-full items-center gap-3 border-b border-border/45 px-4 py-3 text-left transition-colors hover:bg-muted/40 focus-visible:bg-muted/40 focus-visible:outline-hidden"
      data-testid={`home-all-${kind}-${id}`}
      onClick={onClick}
      type="button"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <Icon className="h-4 w-4" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-semibold text-foreground">
          {isDraft ? "Draft" : "Reminder"}
        </span>
        {location ? (
          <ActivityLabel
            isActionRequired={false}
            isDone={false}
            label={location}
          />
        ) : null}
        <span className="block truncate text-sm text-muted-foreground">
          {preview}
        </span>
      </span>
      <span className="shrink-0 text-xs font-medium text-muted-foreground">
        {status}
      </span>
    </button>
  );
}

type InboxListPaneProps = {
  activityEnabled: boolean;
  activeReminderEventIds?: ReadonlySet<string>;
  agentPubkeys?: ReadonlySet<string>;
  activeDraftCount: number;
  draftItems: DraftViewItem[];
  doneSet: ReadonlySet<string>;
  filter: InboxFilter;
  items: InboxItem[];
  onFilterChange: (filter: InboxFilter) => void;
  onDeleteDraft: (draftKey: string) => void;
  onMarkRead: (itemId: string) => void;
  onMarkUnread: (itemId: string) => void;
  onOpenDirect: (item: InboxItem) => void;
  onRemindLater: (item: InboxItem) => void;
  onSelect: (itemId: string) => void;
  onSelectDraft: (draftKey: string) => void;
  onSelectReminder: (reminderId: string) => void;
  onUnreadOnlyChange: (checked: boolean) => void;
  selectedConversationId: string | null;
  selectedDraftKey: string | null;
  showRightDivider?: boolean;
  dueReminderCount: number;
  reminderPubkey?: string;
  reminders: readonly Reminder[];
  selectedReminderId: string | null;
  unreadOnly: boolean;
};

export function InboxListPane({
  activityEnabled,
  activeReminderEventIds,
  agentPubkeys,
  activeDraftCount,
  draftItems,
  doneSet,
  filter,
  items,
  onFilterChange,
  onDeleteDraft,
  onMarkRead,
  onMarkUnread,
  onOpenDirect,
  onRemindLater,
  onSelect,
  onSelectDraft,
  onSelectReminder,
  onUnreadOnlyChange,
  selectedConversationId,
  selectedDraftKey,
  showRightDivider = false,
  dueReminderCount,
  reminderPubkey,
  reminders,
  selectedReminderId,
  unreadOnly,
}: InboxListPaneProps) {
  const filterOptions = activityEnabled
    ? ACTIVITY_FILTER_OPTIONS
    : FILTER_OPTIONS;
  const activeFilter = filterOptions.find((option) => option.value === filter);
  const viewLabel = activityEnabled ? "activity" : "inbox";
  const isReminders = filter === "reminders";
  const isDrafts = filter === "drafts";
  const inboxStatusLabel =
    dueReminderCount > 0
      ? `${dueReminderCount} due reminder${dueReminderCount === 1 ? "" : "s"}`
      : activeDraftCount > 0
        ? `${activeDraftCount} active draft${activeDraftCount === 1 ? "" : "s"}`
        : null;
  const scrollRef = React.useRef<HTMLDivElement>(null);
  const activityRows = React.useMemo(
    () =>
      buildActivityListRows({
        drafts: unreadOnly ? [] : draftItems,
        items,
        reminders: unreadOnly ? [] : reminders,
      }),
    [draftItems, items, reminders, unreadOnly],
  );
  const reminderSources = useReminderSources(reminders);
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

  const renderItem = (item: InboxItem) => {
    const isSelected = item.conversationId === selectedConversationId;
    const isDone = doneSet.has(item.id);
    const hasActiveReminder = activeReminderEventIds?.has(item.id) ?? false;
    const hasChannelTarget = Boolean(item.item.channelId);
    const typeLabel = getInboxTypeLabel(item);
    const isSenderAgent =
      agentPubkeys?.has(normalizePubkey(item.item.pubkey)) === true;
    const profileRole = isSenderAgent ? "bot" : undefined;
    const rowHighlightColor = isSelected
      ? "color-mix(in srgb, hsl(var(--background)) 70%, hsl(var(--muted)) 30%)"
      : "color-mix(in srgb, hsl(var(--background)) 75%, hsl(var(--muted)) 25%)";
    const handleRowContentClick = (event: React.MouseEvent<HTMLElement>) => {
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest("[data-inbox-profile-trigger]")
      ) {
        return;
      }
      onSelect(item.id);
    };
    const row = (
      <div
        aria-current={isSelected ? "true" : undefined}
        className="group/inbox-item relative"
        data-testid={`home-inbox-item-${item.id}`}
        style={
          {
            "--inbox-row-highlight-bg": rowHighlightColor,
          } as React.CSSProperties
        }
      >
        <button
          aria-label={`Open inbox item from ${item.senderLabel}`}
          className="absolute inset-0 z-0 block w-full border-l border-l-transparent text-left"
          onClick={() => onSelect(item.id)}
          type="button"
        >
          <span
            aria-hidden="true"
            className={cn(
              "pointer-events-none absolute inset-y-0 left-0 right-0 transition-colors",
              isSelected
                ? "bg-[var(--inbox-row-highlight-bg)]"
                : "group-hover/inbox-item:bg-[var(--inbox-row-highlight-bg)] group-focus-within/inbox-item:bg-[var(--inbox-row-highlight-bg)] group-active/inbox-item:bg-muted/40",
            )}
          />
        </button>

        {/* biome-ignore lint/a11y: The sibling full-row button provides keyboard/screen-reader row activation; this wrapper delegates pointer selection while allowing nested profile triggers. */}
        <div
          className="relative z-10 block w-full cursor-pointer px-3 py-4 text-left"
          onClick={handleRowContentClick}
        >
          <div className="flex min-w-0 items-start gap-2.5">
            <div
              className="relative shrink-0"
              data-inbox-profile-trigger="true"
            >
              <UserProfilePopover
                botIdenticonValue={item.senderLabel}
                pubkey={item.item.pubkey}
                role={profileRole}
                triggerElement="span"
              >
                <span
                  className="inline-flex shrink-0 rounded-full focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                  data-testid={`home-inbox-avatar-${item.id}`}
                >
                  <UserAvatar
                    avatarUrl={item.avatarUrl}
                    className="h-9 w-9"
                    displayName={item.senderLabel}
                    size="md"
                  />
                </span>
              </UserProfilePopover>
            </div>

            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 items-start gap-2">
                <span
                  className="flex min-w-0 flex-1 items-start leading-4"
                  data-inbox-profile-trigger="true"
                >
                  <UserProfilePopover
                    botIdenticonValue={item.senderLabel}
                    pubkey={item.item.pubkey}
                    role={profileRole}
                    triggerElement="span"
                  >
                    <span className="block max-w-full truncate rounded text-sm font-semibold leading-4 text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring">
                      {item.senderLabel}
                    </span>
                  </UserProfilePopover>
                </span>
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
        </div>

        <div className="pointer-events-none absolute right-3 top-2 z-10 flex items-center gap-0.5 rounded-full bg-[var(--inbox-row-highlight-bg)] p-1 opacity-0 transition-opacity duration-150 ease-out group-hover/inbox-item:pointer-events-auto group-hover/inbox-item:opacity-100 group-focus-within/inbox-item:pointer-events-auto group-focus-within/inbox-item:opacity-100">
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
      <TopChromeInsetHeader flush transparent>
        <div className="px-5 py-2">
          <div className="flex min-h-9 w-full min-w-0 items-center justify-between gap-3">
            <div className="order-2 ml-auto flex shrink-0 items-center justify-end">
              <Popover>
                <PopoverTrigger asChild>
                  <button
                    aria-label={`${activityEnabled ? "Activity" : "Inbox"} options`}
                    className={cn(INBOX_HEADER_ICON_BUTTON_CLASS, "-mr-4")}
                    data-testid="inbox-options-trigger"
                    type="button"
                  >
                    <Ellipsis className="h-4 w-4" />
                  </button>
                </PopoverTrigger>
                <PopoverContent align="end" className="w-60 p-2">
                  <div
                    className={cn(
                      "flex min-h-9 items-center justify-between gap-3 rounded-lg px-2 py-1.5",
                      (isReminders || isDrafts) && "opacity-50",
                    )}
                  >
                    <label
                      className="text-sm font-medium text-foreground"
                      htmlFor="inbox-unread-only-switch"
                    >
                      {activityEnabled ? "Show unread only" : "Show unread"}
                    </label>
                    <Switch
                      checked={unreadOnly}
                      className="shadow-none [&>span]:shadow-none"
                      data-testid="inbox-unread-only-toggle"
                      disabled={isReminders || isDrafts}
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
            </div>
            <div className="order-1 flex shrink-0 items-center justify-start">
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button
                    aria-label={`Filter ${viewLabel}: ${activeFilter?.label ?? "All"}${inboxStatusLabel ? `. ${inboxStatusLabel}` : ""}`}
                    className={cn(
                      INBOX_HEADER_ICON_BUTTON_CLASS,
                      "relative -ml-2 w-auto gap-1 px-2 text-sm font-medium text-foreground",
                    )}
                    data-testid="inbox-filter-trigger"
                    type="button"
                  >
                    <span>{activeFilter?.label ?? "All"}</span>
                    <ChevronDown className="h-4 w-4 text-muted-foreground" />
                    {!activityEnabled && dueReminderCount > 0 ? (
                      <span
                        aria-hidden="true"
                        className="absolute right-1.5 top-0 h-1.5 w-1.5 rounded-full bg-primary ring-2 ring-background"
                        data-testid="inbox-reminder-badge"
                      />
                    ) : !activityEnabled && activeDraftCount > 0 ? (
                      <span
                        aria-hidden="true"
                        className="absolute right-1.5 top-0 h-1.5 w-1.5 rounded-full bg-primary ring-2 ring-background"
                        data-testid="inbox-draft-badge"
                      />
                    ) : null}
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start" className="w-48">
                  <DropdownMenuRadioGroup
                    onValueChange={(value) =>
                      onFilterChange(value as InboxFilter)
                    }
                    value={filter}
                  >
                    {filterOptions.map((option) => (
                      <DropdownMenuRadioItem
                        key={option.value}
                        value={option.value}
                      >
                        <span className="flex flex-1 items-center justify-between gap-2">
                          {option.label}
                          {option.value === "reminders" &&
                          (activityEnabled
                            ? reminders.length
                            : dueReminderCount) > 0 ? (
                            <span
                              className="inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground"
                              data-testid="inbox-reminder-badge-option"
                            >
                              {activityEnabled
                                ? reminders.length
                                : dueReminderCount}
                            </span>
                          ) : option.value === "drafts" &&
                            activeDraftCount > 0 ? (
                            <span
                              className="inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-2xs font-semibold leading-none text-primary-foreground"
                              data-testid="inbox-draft-badge-option"
                            >
                              {activeDraftCount}
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
          className="-mt-13 flex min-h-0 flex-1 flex-col overflow-hidden pt-13"
          data-testid="home-inbox-reminders"
        >
          {reminderPubkey ? (
            <RemindersPanel
              includeDone={!activityEnabled}
              onSelectReminder={activityEnabled ? onSelectReminder : undefined}
              presentation={activityEnabled ? "activity-list" : "card"}
              pubkey={reminderPubkey}
              selectedReminderId={selectedReminderId}
            />
          ) : null}
        </div>
      ) : isDrafts ? (
        <div
          className="-mt-13 flex min-h-0 flex-1 flex-col overflow-hidden pt-13"
          data-testid="home-inbox-drafts"
        >
          <DraftsPanel
            items={draftItems}
            onDeleteDraft={onDeleteDraft}
            onSelectDraft={onSelectDraft}
            selectedDraftKey={selectedDraftKey}
          />
        </div>
      ) : (
        <div
          className="-mt-13 min-h-0 flex-1 overflow-y-auto overflow-x-hidden overscroll-contain pt-13"
          data-testid="home-inbox-list"
          ref={scrollRef}
        >
          {activityEnabled && filter === "all" && activityRows.length > 0 ? (
            <VirtualizedList
              estimateSize={96}
              getItemKey={(row) => row.key}
              items={activityRows}
              renderItem={(row) => {
                if (row.kind === "inbox") return renderItem(row.item);

                if (row.kind === "reminder") {
                  const source = reminderSources.get(row.reminder.id);
                  return (
                    <PersonalItemRow
                      id={row.reminder.id}
                      kind="reminders"
                      location={
                        source?.channel
                          ? source.channel.channelType === "dm"
                            ? {
                                text: `In DM with ${source.channelLabel}`,
                                channelLabel: null,
                              }
                            : { text: "In", channelLabel: source.channelLabel }
                          : null
                      }
                      onClick={() => {
                        onSelectReminder(row.reminder.id);
                        onFilterChange("reminders");
                      }}
                      preview={
                        row.reminder.content.target?.preview ||
                        row.reminder.content.note ||
                        "Reminder"
                      }
                      status={formatReminderStatus(row.reminder.notBefore)}
                    />
                  );
                }

                const { entry, source } = row.item;
                return (
                  <PersonalItemRow
                    id={entry.key}
                    kind="drafts"
                    location={
                      source.channel
                        ? source.channel.channelType === "dm"
                          ? {
                              text: `In DM with ${source.label}`,
                              channelLabel: null,
                            }
                          : { text: "In", channelLabel: source.label }
                        : null
                    }
                    onClick={() => {
                      onSelectDraft(entry.key);
                      onFilterChange("drafts");
                    }}
                    preview={getDraftPreview(entry.draft)}
                    status="Draft saved"
                  />
                );
              }}
              scrollRef={scrollRef}
            />
          ) : items.length === 0 ? (
            <div className="flex h-full min-h-64 items-center justify-center px-6 text-center">
              <div>
                <p className="text-sm font-medium text-foreground">
                  {activityEnabled
                    ? unreadOnly
                      ? ACTIVITY_UNREAD_EMPTY_STATE_TITLES[filter]
                      : ACTIVITY_EMPTY_STATE_TITLES[filter]
                    : unreadOnly
                      ? "No unread messages"
                      : "No messages found"}
                </p>
                <p className="mt-1 text-sm text-muted-foreground">
                  {activityEnabled
                    ? unreadOnly
                      ? "Turn off Show unread only to see read activity."
                      : filter === "all"
                        ? "New activity will appear here."
                        : "Switch back to All to see other activity."
                    : unreadOnly
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
