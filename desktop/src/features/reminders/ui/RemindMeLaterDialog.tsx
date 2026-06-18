import { CalendarClock, Clock } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useReminderMutations } from "@/features/reminders/hooks";
import {
  parseCustomDateTime,
  TIME_PRESETS,
  todayDateString,
} from "@/features/reminders/lib/timePresets";
import type { ReminderTarget } from "@/features/reminders/lib/reminderTypes";
import { useIdentityQuery } from "@/shared/api/hooks";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

export function RemindMeLaterDialog({
  open,
  onOpenChange,
  target,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  target: ReminderTarget | null;
}) {
  const pubkey = useIdentityQuery().data?.pubkey ?? "";
  const { create } = useReminderMutations(pubkey);
  const [note, setNote] = React.useState("");
  const [customDate, setCustomDate] = React.useState(todayDateString);
  const [customTime, setCustomTime] = React.useState("09:00");
  const customTimestamp = parseCustomDateTime(customDate, customTime);

  const submit = (notBefore: number) => {
    if (!target || create.isPending) return;
    create.mutate(
      { target, notBefore, note: note || undefined },
      {
        onSuccess: () => {
          toast.success("Reminder set");
          onOpenChange(false);
          setNote("");
        },
        onError: () => toast.error("Failed to create reminder"),
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[400px]">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Clock className="h-4 w-4" />
            Remind me later
          </DialogTitle>
          <DialogDescription>
            Choose when you want to be reminded about this message.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-2">
          {TIME_PRESETS.map((preset) => (
            <Button
              key={preset.label}
              variant="outline"
              className="justify-start"
              disabled={create.isPending}
              onClick={() => submit(preset.getTimestamp())}
            >
              {preset.label}
            </Button>
          ))}
        </div>

        <div className="space-y-3 border-t pt-3">
          <p className="flex items-center gap-2 text-sm font-medium">
            <CalendarClock className="h-4 w-4" />
            Custom date & time
          </p>
          <div className="flex gap-2">
            <Input
              aria-label="Reminder date"
              className="flex-1"
              min={todayDateString()}
              onChange={(e) => setCustomDate(e.target.value)}
              type="date"
              value={customDate}
            />
            <Input
              aria-label="Reminder time"
              className="w-[120px]"
              onChange={(e) => setCustomTime(e.target.value)}
              type="time"
              value={customTime}
            />
          </div>
          <Button
            className="w-full"
            disabled={create.isPending || customTimestamp === null}
            onClick={() => {
              if (customTimestamp === null) return;
              submit(customTimestamp);
            }}
            variant="default"
          >
            Set reminder
          </Button>
        </div>

        <div className="space-y-2">
          <label
            htmlFor="reminder-note"
            className="text-sm font-medium text-muted-foreground"
          >
            Note (optional)
          </label>
          <Textarea
            id="reminder-note"
            placeholder="Add a note..."
            value={note}
            onChange={(e) => setNote(e.target.value)}
            rows={2}
            className="resize-none"
          />
        </div>

        <DialogFooter>
          <Button
            variant="ghost"
            onClick={() => onOpenChange(false)}
            disabled={create.isPending}
          >
            Cancel
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
