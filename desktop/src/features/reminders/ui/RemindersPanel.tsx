import { ArrowLeft, Bell, Check, Clock, ExternalLink, X } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import {
  useRemindersQuery,
  useReminderMutations,
} from "@/features/reminders/hooks";
import { groupReminders } from "@/features/reminders/lib/reminderFilters";
import {
  hasNavigableTarget,
  resolveReminderDestination,
} from "@/features/reminders/lib/reminderNavigation";
import type { Reminder } from "@/features/reminders/lib/reminderTypes";
import { SnoozeMenu } from "@/features/reminders/ui/SnoozeMenu";
import { resolveChannelDisplayLabel } from "@/features/sidebar/lib/channelLabels";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel } from "@/shared/api/types";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const UNKNOWN_CHANNEL_LABEL = "Unknown channel";

/** Author identity + source channel resolved for a reminder's target. */
export type ReminderSource = {
  authorLabel: string;
  avatarUrl: string | null;
  channel: Channel | null;
  channelLabel: string;
};

export function useReminderSources(reminders: readonly Reminder[]) {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const channelsQuery = useChannelsQuery();
  const channels = channelsQuery.data;
  const authorPubkeys = React.useMemo(
    () =>
      reminders
        .map((reminder) => reminder.content.target?.authorPubkey)
        .filter((authorPubkey): authorPubkey is string => !!authorPubkey),
    [reminders],
  );
  const usersBatchQuery = useUsersBatchQuery(authorPubkeys);
  const profiles: UserProfileLookup | undefined =
    usersBatchQuery.data?.profiles;

  return React.useMemo(() => {
    const channelsById = new Map(
      (channels ?? []).map((channel) => [channel.id, channel]),
    );
    const map = new Map<string, ReminderSource>();
    for (const reminder of reminders) {
      const target = reminder.content.target;
      if (!hasNavigableTarget(target)) continue;
      const channel = channelsById.get(target.channelId);
      map.set(reminder.id, {
        authorLabel: resolveUserLabel({
          currentPubkey,
          profiles,
          pubkey: target.authorPubkey,
        }),
        avatarUrl:
          profiles?.[normalizePubkey(target.authorPubkey)]?.avatarUrl ?? null,
        channel: channel ?? null,
        channelLabel: channel
          ? resolveChannelDisplayLabel(channel, currentPubkey, profiles)
          : UNKNOWN_CHANNEL_LABEL,
      });
    }
    return map;
  }, [channels, currentPubkey, profiles, reminders]);
}

function formatRelativeTime(timestamp: number): string {
  const now = Math.floor(Date.now() / 1_000);
  const diff = timestamp - now;

  if (diff < 0) {
    const absDiff = Math.abs(diff);
    if (absDiff < 60) return "just now";
    if (absDiff < 3600) return `${Math.floor(absDiff / 60)}m overdue`;
    if (absDiff < 86400) return `${Math.floor(absDiff / 3600)}h overdue`;
    return `${Math.floor(absDiff / 86400)}d overdue`;
  }

  if (diff < 60) return "in less than a minute";
  if (diff < 3600) return `in ${Math.floor(diff / 60)}m`;
  if (diff < 86400) return `in ${Math.floor(diff / 3600)}h`;
  return `in ${Math.floor(diff / 86400)}d`;
}

function ReminderRow({
  isSelected = false,
  presentation = "card",
  reminder,
  pubkey,
  source,
  onNavigate,
  onSelect,
}: {
  isSelected?: boolean;
  presentation?: "activity-list" | "card";
  reminder: Reminder;
  pubkey: string;
  source: ReminderSource | null;
  onNavigate: (reminder: Reminder) => void;
  onSelect?: (reminder: Reminder) => void;
}) {
  const { complete, snooze, cancel } = useReminderMutations(pubkey);
  const isDone = reminder.content.status === "done";
  const isActing = complete.isPending || snooze.isPending || cancel.isPending;
  const isNavigable = hasNavigableTarget(reminder.content.target);

  const handleComplete = () => {
    complete.mutate(reminder, {
      onSuccess: () => toast.success("Reminder completed"),
      onError: () => toast.error("Failed to complete reminder"),
    });
  };

  const handleSnooze = (notBefore: number) => {
    snooze.mutate(
      { reminder, notBefore },
      {
        onSuccess: () => toast.success("Reminder snoozed"),
        onError: () => toast.error("Failed to snooze reminder"),
      },
    );
  };

  const handleCancel = () => {
    cancel.mutate(reminder, {
      onSuccess: () => toast.success("Reminder cancelled"),
      onError: () => toast.error("Failed to cancel reminder"),
    });
  };

  const isOverdue =
    !isDone && reminder.notBefore
      ? reminder.notBefore <= Math.floor(Date.now() / 1_000)
      : false;
  const isActivityList = presentation === "activity-list";

  return (
    <div
      className={cn(
        "flex items-start gap-3 transition-colors",
        isActivityList
          ? "border-b border-border/45 px-4 py-4 hover:bg-muted/40 focus-within:bg-muted/40"
          : "rounded-md border p-3",
        isActivityList && isSelected && "bg-muted/40",
      )}
      data-testid={`home-reminder-item-${reminder.id}`}
    >
      {isActivityList ? (
        <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
          <Bell className="h-4 w-4" />
        </span>
      ) : null}
      <button
        className="flex min-w-0 flex-1 flex-col items-start gap-1 text-left enabled:hover:opacity-80 disabled:cursor-default"
        disabled={!isActivityList && !isNavigable}
        onClick={() => {
          if (isActivityList) onSelect?.(reminder);
          else if (isNavigable) onNavigate(reminder);
        }}
        type="button"
      >
        {source ? (
          <div className="flex min-w-0 max-w-full items-center gap-1.5 text-xs text-muted-foreground">
            <UserAvatar
              avatarUrl={source.avatarUrl}
              className="h-4 w-4 shrink-0"
              displayName={source.authorLabel}
              size="xs"
            />
            <span className="truncate font-medium text-foreground">
              {source.authorLabel}
            </span>
            <span className="shrink-0">in</span>
            <span className="truncate">{source.channelLabel}</span>
          </div>
        ) : null}
        <p className="max-w-full truncate text-sm font-medium">
          {reminder.content.target?.preview ||
            reminder.content.note ||
            "Reminder"}
        </p>
        {reminder.content.target && reminder.content.note ? (
          <p className="max-w-full truncate text-xs text-muted-foreground">
            {reminder.content.note}
          </p>
        ) : null}
        {reminder.notBefore ? (
          <p
            className={`text-xs ${isOverdue ? "font-medium text-destructive" : "text-muted-foreground"}`}
          >
            <Clock className="mr-1 inline h-3 w-3" />
            {formatRelativeTime(reminder.notBefore)}
          </p>
        ) : null}
      </button>
      {isDone || isActivityList ? null : (
        <div className="flex shrink-0 items-center gap-1">
          <Button
            className="h-7 w-7 p-0"
            disabled={isActing}
            onClick={handleComplete}
            size="sm"
            title="Complete"
            type="button"
            variant="ghost"
          >
            <Check className="h-4 w-4" />
          </Button>
          <SnoozeMenu disabled={isActing} onSnooze={handleSnooze} />
          <Button
            className="h-7 w-7 p-0"
            disabled={isActing}
            onClick={handleCancel}
            size="sm"
            title="Cancel"
            type="button"
            variant="ghost"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      )}
    </div>
  );
}

/**
 * Renders a user's reminders as grouped rows. `includeDone` adds a Completed
 * group (used by the inbox Reminders view); omit it for pending-only surfaces.
 */
export function RemindersPanel({
  pubkey,
  includeDone = false,
  onSelectReminder,
  presentation = "card",
  selectedReminderId,
}: {
  pubkey: string;
  includeDone?: boolean;
  onSelectReminder?: (reminderId: string) => void;
  presentation?: "activity-list" | "card";
  selectedReminderId?: string | null;
}) {
  const remindersQuery = useRemindersQuery(pubkey);
  const reminders = remindersQuery.data;
  const { goChannel } = useAppNavigation();
  const sources = useReminderSources(reminders ?? []);

  const handleNavigate = React.useCallback(
    async (reminder: Reminder) => {
      const destination = await resolveReminderDestination(
        reminder.content.target,
      );
      if (!destination) {
        return;
      }
      void goChannel(destination.channelId, {
        messageId: destination.messageId,
        threadRootId: destination.threadRootId,
      });
    },
    [goChannel],
  );

  const groups = React.useMemo(
    () => groupReminders(reminders ?? [], includeDone),
    [reminders, includeDone],
  );
  const activityReminders = React.useMemo(
    () => groups.flatMap((group) => group.reminders),
    [groups],
  );

  if (remindersQuery.isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <p className="text-sm text-muted-foreground">Loading reminders...</p>
      </div>
    );
  }

  if (groups.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 p-8">
        <Bell className="h-8 w-8 text-muted-foreground/50" />
        <p className="text-sm text-muted-foreground">No reminders</p>
        <p className="text-xs text-muted-foreground/70">
          Use "Remind me later" on any message to create one.
        </p>
      </div>
    );
  }

  if (presentation === "activity-list") {
    return (
      <div className="min-h-0 flex-1 overflow-y-auto">
        {activityReminders.map((reminder) => (
          <ReminderRow
            isSelected={reminder.id === selectedReminderId}
            key={reminder.id}
            onNavigate={handleNavigate}
            onSelect={(selected) => onSelectReminder?.(selected.id)}
            presentation="activity-list"
            pubkey={pubkey}
            reminder={reminder}
            source={sources.get(reminder.id) ?? null}
          />
        ))}
      </div>
    );
  }

  return (
    <div className="flex-1 space-y-4 overflow-y-auto p-4">
      {groups.map((group) => (
        <div key={group.label} className="space-y-2">
          <h3 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
            {group.label}
          </h3>
          {group.reminders.map((r) => (
            <ReminderRow
              key={r.id}
              onNavigate={handleNavigate}
              pubkey={pubkey}
              reminder={r}
              source={sources.get(r.id) ?? null}
            />
          ))}
        </div>
      ))}
    </div>
  );
}

export function ReminderDetailPane({
  onBack,
  pubkey,
  reminder,
}: {
  onBack?: () => void;
  pubkey: string;
  reminder: Reminder | null;
}) {
  const { goChannel } = useAppNavigation();
  const reminderList = React.useMemo(
    () => (reminder ? [reminder] : []),
    [reminder],
  );
  const sources = useReminderSources(reminderList);
  const { complete, snooze, cancel } = useReminderMutations(pubkey);

  if (!reminder) {
    return (
      <section className="flex min-h-0 min-w-0 flex-col bg-background">
        <TopChromeInsetHeader flush>
          <div className="flex min-h-9 items-center px-4 py-2">
            <span className="text-sm font-semibold">Reminder</span>
          </div>
        </TopChromeInsetHeader>
        <div className="flex flex-1 items-center justify-center p-8 text-sm text-muted-foreground">
          Select a reminder
        </div>
      </section>
    );
  }

  const source = sources.get(reminder.id) ?? null;
  const isDone = reminder.content.status === "done";
  const isActing = complete.isPending || snooze.isPending || cancel.isPending;
  const isNavigable = hasNavigableTarget(reminder.content.target);
  const preview =
    reminder.content.target?.preview || reminder.content.note || "Reminder";

  const handleNavigate = async () => {
    const destination = await resolveReminderDestination(
      reminder.content.target,
    );
    if (!destination) return;
    void goChannel(destination.channelId, {
      messageId: destination.messageId,
      threadRootId: destination.threadRootId,
    });
  };

  return (
    <section
      className="flex min-h-0 min-w-0 flex-col bg-background"
      data-testid="home-reminder-detail"
    >
      <TopChromeInsetHeader flush>
        <div className="flex min-h-9 items-center gap-2 px-4 py-2">
          {onBack ? (
            <Button
              aria-label="Back to reminders"
              className="h-8 w-8 p-0"
              onClick={onBack}
              size="icon"
              type="button"
              variant="ghost"
            >
              <ArrowLeft className="h-4 w-4" />
            </Button>
          ) : null}
          <span className="text-sm font-semibold">Reminder</span>
        </div>
      </TopChromeInsetHeader>

      <div className="min-h-0 flex-1 overflow-y-auto px-6 py-8">
        <div className="mx-auto max-w-2xl">
          {source ? (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <UserAvatar
                avatarUrl={source.avatarUrl}
                className="h-6 w-6"
                displayName={source.authorLabel}
                size="sm"
              />
              <span className="font-medium text-foreground">
                {source.authorLabel}
              </span>
              <span>in</span>
              <span>
                {source.channel?.channelType === "dm" ? "" : "#"}
                {source.channelLabel}
              </span>
            </div>
          ) : null}

          <p className="mt-5 whitespace-pre-wrap text-base leading-6 text-foreground">
            {preview}
          </p>
          {reminder.content.target && reminder.content.note ? (
            <div className="mt-5 border-l-2 border-border pl-4">
              <p className="text-xs font-medium uppercase text-muted-foreground">
                Note
              </p>
              <p className="mt-1 whitespace-pre-wrap text-sm text-foreground">
                {reminder.content.note}
              </p>
            </div>
          ) : null}

          {reminder.notBefore ? (
            <div className="mt-6 flex items-center gap-2 text-sm text-muted-foreground">
              <Clock className="h-4 w-4" />
              <span>{formatRelativeTime(reminder.notBefore)}</span>
            </div>
          ) : null}

          <div className="mt-8 flex flex-wrap items-center gap-2">
            <Button
              disabled={!isNavigable}
              onClick={() => void handleNavigate()}
              size="sm"
              type="button"
              variant="outline"
            >
              <ExternalLink className="h-4 w-4" />
              Open message
            </Button>
            {isDone ? null : (
              <>
                <Button
                  disabled={isActing}
                  onClick={() =>
                    complete.mutate(reminder, {
                      onSuccess: () => toast.success("Reminder completed"),
                      onError: () => toast.error("Failed to complete reminder"),
                    })
                  }
                  size="sm"
                  type="button"
                >
                  <Check className="h-4 w-4" />
                  Complete
                </Button>
                <SnoozeMenu
                  disabled={isActing}
                  onSnooze={(notBefore) =>
                    snooze.mutate(
                      { reminder, notBefore },
                      {
                        onSuccess: () => toast.success("Reminder snoozed"),
                        onError: () => toast.error("Failed to snooze reminder"),
                      },
                    )
                  }
                />
                <Button
                  disabled={isActing}
                  onClick={() =>
                    cancel.mutate(reminder, {
                      onSuccess: () => toast.success("Reminder cancelled"),
                      onError: () => toast.error("Failed to cancel reminder"),
                    })
                  }
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  <X className="h-4 w-4" />
                  Cancel
                </Button>
              </>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
