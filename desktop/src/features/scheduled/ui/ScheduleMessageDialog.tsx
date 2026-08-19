import * as React from "react";
import { CalendarClock, ListChecks } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  datetimeLocalToIso,
  defaultScheduleDatetime,
  formatDeliveryTime,
  SCHEDULE_PRESETS,
  unixToDatetimeLocal,
} from "@/features/scheduled/lib/scheduledMessages";
import { useScheduleMessageMutation } from "@/features/scheduled/useScheduledMessages";
import type { ScheduledMessage } from "@/shared/api/scheduledMessages";
import { cn } from "@/shared/lib/cn";

type ScheduleMessageDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  channelId: string | null;
  channelName: string;
  content: string;
  mentionPubkeys?: string[];
  parentEventId?: string | null;
  /** Fired after a successful enqueue (the composer clears itself). */
  onScheduled?: (message: ScheduledMessage) => void;
  /** Switch to the "Scheduled messages" management view. */
  onViewScheduled?: () => void;
};

export function ScheduleMessageDialog({
  open,
  onOpenChange,
  channelId,
  channelName,
  content,
  mentionPubkeys = [],
  parentEventId = null,
  onScheduled,
  onViewScheduled,
}: ScheduleMessageDialogProps) {
  const scheduleMutation = useScheduleMessageMutation();
  const [datetimeValue, setDatetimeValue] = React.useState<string>(() =>
    defaultScheduleDatetime(),
  );
  const [validationError, setValidationError] = React.useState<string | null>(
    null,
  );

  // Reset to a fresh default whenever the dialog is re-opened.
  React.useEffect(() => {
    if (open) {
      setDatetimeValue(defaultScheduleDatetime());
      setValidationError(null);
    }
  }, [open]);

  const canSchedule = content.trim().length > 0 && !scheduleMutation.isPending;

  const handleSchedule = React.useCallback(async () => {
    const iso = datetimeLocalToIso(datetimeValue);
    if (iso == null) {
      setValidationError("Choose a delivery date and time.");
      return;
    }
    const timestamp = Math.floor(new Date(iso).getTime() / 1000);
    if (timestamp <= Math.floor(Date.now() / 1000)) {
      setValidationError("Delivery time must be in the future.");
      return;
    }
    setValidationError(null);
    if (channelId == null) {
      setValidationError("Choose a channel first.");
      return;
    }
    try {
      const scheduled = await scheduleMutation.mutateAsync({
        channelId,
        content: content.trim(),
        replyTo: parentEventId,
        mentions: mentionPubkeys,
        scheduledAt: iso,
      });
      toast.success(`Scheduled for ${formatDeliveryTime(timestamp)}`);
      onOpenChange(false);
      onScheduled?.(scheduled);
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to schedule the message.",
      );
    }
  }, [
    channelId,
    content,
    datetimeValue,
    mentionPubkeys,
    onOpenChange,
    onScheduled,
    parentEventId,
    scheduleMutation,
  ]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <CalendarClock aria-hidden className="size-4" />
            Schedule for later
          </DialogTitle>
          <DialogDescription>
            Deliver a message to{" "}
            <span className="font-medium">#{channelName}</span> at a time that
            doesn't interrupt anyone.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2">
          <div className="space-y-2">
            <label
              className="text-sm font-medium text-foreground"
              htmlFor="scheduled-delivery-time"
            >
              Delivery time
            </label>
            <Input
              aria-invalid={validationError != null}
              className={cn(
                "tabular-nums",
                validationError != null &&
                  "border-destructive focus-visible:ring-destructive",
              )}
              data-testid="scheduled-delivery-time"
              id="scheduled-delivery-time"
              onChange={(event) => setDatetimeValue(event.target.value)}
              type="datetime-local"
              value={datetimeValue}
            />
            {validationError != null ? (
              <p
                className="text-xs text-destructive"
                data-testid="scheduled-time-error"
                role="alert"
              >
                {validationError}
              </p>
            ) : null}
            <div className="flex flex-wrap gap-1.5">
              {SCHEDULE_PRESETS.map((preset) => (
                <Button
                  data-testid={`scheduled-preset-${preset.label.replaceAll(
                    " ",
                    "-",
                  )}`}
                  key={preset.label}
                  onClick={() => {
                    setValidationError(null);
                    setDatetimeValue(
                      unixToDatetimeLocal(
                        Math.floor((Date.now() + preset.deltaMs) / 1000),
                      ),
                    );
                  }}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  {preset.label}
                </Button>
              ))}
            </div>
          </div>

          <div className="space-y-1">
            <span className="text-sm font-medium text-foreground">Message</span>
            <p className="line-clamp-3 whitespace-pre-wrap rounded-lg bg-muted/50 px-3 py-2 text-sm text-foreground/90">
              {content.trim()}
            </p>
          </div>
        </div>

        <DialogFooter className="gap-2 sm:justify-between">
          <Button
            onClick={onViewScheduled}
            size="sm"
            type="button"
            variant="ghost"
          >
            <ListChecks aria-hidden className="size-4" />
            View scheduled
          </Button>
          <div className="flex items-center gap-2">
            <Button
              onClick={() => onOpenChange(false)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              data-testid="schedule-confirm"
              disabled={!canSchedule}
              onClick={() => void handleSchedule()}
              type="button"
            >
              {scheduleMutation.isPending ? "Scheduling…" : "Schedule"}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
