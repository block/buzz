import { Clock } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { createReminder } from "@/features/reminders/lib/reminderService";
import type { ReminderTarget } from "@/features/reminders/lib/reminderTypes";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Textarea } from "@/shared/ui/textarea";

type TimePreset = {
  label: string;
  getTimestamp: () => number;
};

function getNextWeekday9am(dayOffset: number): number {
  const now = new Date();
  const target = new Date(now);
  target.setDate(target.getDate() + dayOffset);
  target.setHours(9, 0, 0, 0);
  if (target.getTime() <= now.getTime()) {
    target.setDate(target.getDate() + 1);
  }
  return Math.floor(target.getTime() / 1_000);
}

const TIME_PRESETS: TimePreset[] = [
  {
    label: "In 30 minutes",
    getTimestamp: () => Math.floor(Date.now() / 1_000) + 30 * 60,
  },
  {
    label: "In 1 hour",
    getTimestamp: () => Math.floor(Date.now() / 1_000) + 60 * 60,
  },
  {
    label: "In 3 hours",
    getTimestamp: () => Math.floor(Date.now() / 1_000) + 3 * 60 * 60,
  },
  {
    label: "Tomorrow at 9am",
    getTimestamp: () => getNextWeekday9am(1),
  },
  {
    label: "Next Monday at 9am",
    getTimestamp: () => {
      const now = new Date();
      const daysUntilMonday = (8 - now.getDay()) % 7 || 7;
      return getNextWeekday9am(daysUntilMonday);
    },
  },
];

export function RemindMeLaterDialog({
  open,
  onOpenChange,
  target,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  target: ReminderTarget | null;
}) {
  const [note, setNote] = React.useState("");
  const [isSubmitting, setIsSubmitting] = React.useState(false);

  const handleSelect = async (preset: TimePreset) => {
    if (!target || isSubmitting) return;
    setIsSubmitting(true);
    try {
      await createReminder(target, preset.getTimestamp(), note || undefined);
      toast.success("Reminder set");
      onOpenChange(false);
      setNote("");
    } catch (error) {
      toast.error("Failed to create reminder");
      console.error("[RemindMeLaterDialog] create failed:", error);
    } finally {
      setIsSubmitting(false);
    }
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
              disabled={isSubmitting}
              onClick={() => void handleSelect(preset)}
            >
              {preset.label}
            </Button>
          ))}
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
            disabled={isSubmitting}
          >
            Cancel
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
