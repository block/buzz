import { Bell, Check, Clock, X } from "lucide-react";
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
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const UNKNOWN_CHANNEL_LABEL = "Unknown channel";

/** Author identity + source channel resolved for a reminder's target. */
type ReminderSource = {
  authorLabel: string;
  avatarUrl: string | null;
  channelLabel: string;
};

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
  reminder,
  pubkey,
  source,
  onNavigate,
}: {
  reminder: Reminder;
  pubkey: string;
  source: ReminderSource | null;
  onNavigate: (reminder: Reminder) => void;
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

  return (
    <div className="flex items-start gap-3 rounded-md border p-3">
      <button
        className="flex min-w-0 flex-1 flex-col items-start gap-1 text-left enabled:hover:opacity-80 disabled:cursor-default"
        disabled={!isNavigable}
        onClick={isNavigable ? () => onNavigate(reminder) : undefined}
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
      {isDone ? null : (
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
}: {
  pubkey: string;
  includeDone?: boolean;
}) {
  const remindersQuery = useRemindersQuery(pubkey);
  const reminders = remindersQuery.data;
  const { goChannel } = useAppNavigation();
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const channelsQuery = useChannelsQuery();
  const channels = channelsQuery.data;

  const authorPubkeys = React.useMemo(
    () =>
      (reminders ?? [])
        .map((reminder) => reminder.content.target?.authorPubkey)
        .filter((authorPubkey): authorPubkey is string => !!authorPubkey),
    [reminders],
  );
  const usersBatchQuery = useUsersBatchQuery(authorPubkeys);
  const profiles: UserProfileLookup | undefined =
    usersBatchQuery.data?.profiles;

  // Look up each reminder's author + source channel from the live profile and
  // channel queries. Channels/profiles can be missing — reminders outlive the
  // context they were set in (left/archived channel, hidden DM) — so fall back
  // to a resolved-or-truncated author label and a neutral channel label.
  const sources = React.useMemo(() => {
    const channelsById = new Map(
      (channels ?? []).map((channel) => [channel.id, channel]),
    );
    const map = new Map<string, ReminderSource>();
    for (const reminder of reminders ?? []) {
      const target = reminder.content.target;
      if (!hasNavigableTarget(target)) {
        continue;
      }
      const channel = channelsById.get(target.channelId);
      map.set(reminder.id, {
        authorLabel: resolveUserLabel({
          currentPubkey,
          profiles,
          pubkey: target.authorPubkey,
        }),
        avatarUrl:
          profiles?.[normalizePubkey(target.authorPubkey)]?.avatarUrl ?? null,
        channelLabel: channel
          ? resolveChannelDisplayLabel(channel, currentPubkey, profiles)
          : UNKNOWN_CHANNEL_LABEL,
      });
    }
    return map;
  }, [reminders, channels, profiles, currentPubkey]);

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
