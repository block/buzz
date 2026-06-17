import { Bell, Check, Clock, RotateCcw, X } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  cancelReminder,
  completeReminder,
  fetchReminders,
  snoozeReminder,
} from "@/features/reminders/lib/reminderService";
import type { Reminder } from "@/features/reminders/lib/reminderTypes";
import { Button } from "@/shared/ui/button";

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

type ReminderGroup = {
  label: string;
  reminders: Reminder[];
};

function groupReminders(reminders: Reminder[]): ReminderGroup[] {
  const now = Math.floor(Date.now() / 1_000);
  const endOfToday = new Date();
  endOfToday.setHours(23, 59, 59, 999);
  const endOfTodaySecs = Math.floor(endOfToday.getTime() / 1_000);

  const overdue: Reminder[] = [];
  const today: Reminder[] = [];
  const upcoming: Reminder[] = [];

  for (const r of reminders) {
    if (r.content.status !== "pending") continue;
    if (!r.notBefore) continue;
    if (r.notBefore <= now) {
      overdue.push(r);
    } else if (r.notBefore <= endOfTodaySecs) {
      today.push(r);
    } else {
      upcoming.push(r);
    }
  }

  const groups: ReminderGroup[] = [];
  if (overdue.length > 0) groups.push({ label: "Overdue", reminders: overdue });
  if (today.length > 0) groups.push({ label: "Today", reminders: today });
  if (upcoming.length > 0)
    groups.push({ label: "Upcoming", reminders: upcoming });
  return groups;
}

function ReminderRow({
  reminder,
  pubkey,
  onUpdate,
}: {
  reminder: Reminder;
  pubkey: string;
  onUpdate: () => void;
}) {
  const [isActing, setIsActing] = React.useState(false);

  const handleComplete = async () => {
    setIsActing(true);
    try {
      await completeReminder(pubkey, reminder);
      toast.success("Reminder completed");
      onUpdate();
    } catch {
      toast.error("Failed to complete reminder");
    } finally {
      setIsActing(false);
    }
  };

  const handleSnooze = async () => {
    setIsActing(true);
    try {
      const newNotBefore = Math.floor(Date.now() / 1_000) + 3600;
      await snoozeReminder(pubkey, reminder, newNotBefore);
      toast.success("Snoozed for 1 hour");
      onUpdate();
    } catch {
      toast.error("Failed to snooze reminder");
    } finally {
      setIsActing(false);
    }
  };

  const handleCancel = async () => {
    setIsActing(true);
    try {
      await cancelReminder(pubkey, reminder);
      toast.success("Reminder cancelled");
      onUpdate();
    } catch {
      toast.error("Failed to cancel reminder");
    } finally {
      setIsActing(false);
    }
  };

  const isOverdue = reminder.notBefore
    ? reminder.notBefore <= Math.floor(Date.now() / 1_000)
    : false;

  return (
    <div className="flex items-start gap-3 rounded-md border p-3">
      <div className="flex-1 min-w-0">
        <p className="text-sm font-medium truncate">
          {reminder.content.target?.preview ||
            reminder.content.note ||
            "Reminder"}
        </p>
        {reminder.content.target && reminder.content.note ? (
          <p className="text-xs text-muted-foreground mt-0.5 truncate">
            {reminder.content.note}
          </p>
        ) : null}
        {reminder.notBefore ? (
          <p
            className={`text-xs mt-1 ${isOverdue ? "text-destructive font-medium" : "text-muted-foreground"}`}
          >
            <Clock className="inline h-3 w-3 mr-1" />
            {formatRelativeTime(reminder.notBefore)}
          </p>
        ) : null}
      </div>
      <div className="flex items-center gap-1 shrink-0">
        <Button
          size="sm"
          variant="ghost"
          className="h-7 w-7 p-0"
          disabled={isActing}
          onClick={() => void handleComplete()}
          title="Complete"
        >
          <Check className="h-3.5 w-3.5" />
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="h-7 w-7 p-0"
          disabled={isActing}
          onClick={() => void handleSnooze()}
          title="Snooze 1 hour"
        >
          <RotateCcw className="h-3.5 w-3.5" />
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="h-7 w-7 p-0"
          disabled={isActing}
          onClick={() => void handleCancel()}
          title="Cancel"
        >
          <X className="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>
  );
}

export function RemindersPanel({ pubkey }: { pubkey: string }) {
  const [reminders, setReminders] = React.useState<Reminder[]>([]);
  const [isLoading, setIsLoading] = React.useState(true);

  const loadReminders = React.useCallback(async () => {
    try {
      const fetched = await fetchReminders(pubkey);
      setReminders(fetched);
    } catch (error) {
      console.error("[RemindersPanel] fetch failed:", error);
    } finally {
      setIsLoading(false);
    }
  }, [pubkey]);

  React.useEffect(() => {
    void loadReminders();
  }, [loadReminders]);

  // Due-detection: check every 60s and show toast for newly due reminders.
  const lastDueCheckRef = React.useRef<number>(Math.floor(Date.now() / 1_000));
  React.useEffect(() => {
    const interval = window.setInterval(() => {
      const now = Math.floor(Date.now() / 1_000);
      const lastCheck = lastDueCheckRef.current;
      lastDueCheckRef.current = now;

      for (const r of reminders) {
        if (r.content.status !== "pending") continue;
        if (!r.notBefore) continue;
        if (r.notBefore > lastCheck && r.notBefore <= now) {
          toast("Reminder due", {
            description:
              r.content.target?.preview ||
              r.content.note ||
              "A reminder is waiting",
            icon: <Bell className="h-4 w-4" />,
          });
        }
      }
    }, 60_000);

    return () => window.clearInterval(interval);
  }, [reminders]);

  const groups = groupReminders(reminders);
  const pendingCount = reminders.filter(
    (r) => r.content.status === "pending",
  ).length;

  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <p className="text-sm text-muted-foreground">Loading reminders...</p>
      </div>
    );
  }

  if (pendingCount === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 p-8">
        <Bell className="h-8 w-8 text-muted-foreground/50" />
        <p className="text-sm text-muted-foreground">No pending reminders</p>
        <p className="text-xs text-muted-foreground/70">
          Use "Remind me later" on any message to create one.
        </p>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="border-b px-4 py-3">
        <h2 className="text-sm font-semibold flex items-center gap-2">
          <Bell className="h-4 w-4" />
          Reminders
          <span className="text-xs font-normal text-muted-foreground">
            ({pendingCount})
          </span>
        </h2>
      </div>
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {groups.map((group) => (
          <div key={group.label} className="space-y-2">
            <h3 className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
              {group.label}
            </h3>
            {group.reminders.map((r) => (
              <ReminderRow
                key={r.id}
                reminder={r}
                pubkey={pubkey}
                onUpdate={() => void loadReminders()}
              />
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}
