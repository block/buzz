import * as React from "react";
import { CalendarClock, Pencil, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  datetimeLocalToIso,
  formatDeliveryTime,
  unixToDatetimeLocal,
} from "@/features/scheduled/lib/scheduledMessages";
import {
  useCancelScheduledMessageMutation,
  useScheduleMessageMutation,
  useScheduledMessagesQuery,
} from "@/features/scheduled/useScheduledMessages";
import type { ScheduledMessage } from "@/shared/api/scheduledMessages";

type ScheduledMessagesDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

function ScheduledMessageRow({
  message,
  channelLabel,
  onCancel,
  onSaveEdit,
}: {
  message: ScheduledMessage;
  channelLabel: string | null;
  onCancel: (id: string) => void;
  onSaveEdit: (message: ScheduledMessage, iso: string) => void;
}) {
  const [editing, setEditing] = React.useState(false);
  const [datetimeValue, setDatetimeValue] = React.useState(() =>
    unixToDatetimeLocal(message.scheduledAt),
  );
  const [error, setError] = React.useState<string | null>(null);

  const commitEdit = () => {
    const iso = datetimeLocalToIso(datetimeValue);
    if (iso == null) {
      setError("Choose a delivery date and time.");
      return;
    }
    if (
      Math.floor(new Date(iso).getTime() / 1000) <=
      Math.floor(Date.now() / 1000)
    ) {
      setError("Delivery time must be in the future.");
      return;
    }
    setError(null);
    onSaveEdit(message, iso);
    setEditing(false);
  };

  return (
    <li className="flex flex-col gap-2 rounded-lg border border-border/60 bg-background/60 px-3 py-2.5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="truncate text-sm font-medium text-foreground">
            {channelLabel ? `#${channelLabel}` : "Channel"}
          </p>
          <p className="line-clamp-2 whitespace-pre-wrap text-sm text-foreground/80">
            {message.content}
          </p>
          <p className="mt-0.5 text-xs text-muted-foreground">
            <CalendarClock aria-hidden className="mr-1 inline size-3" />
            {formatDeliveryTime(message.scheduledAt)}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <Button
            aria-label="Reschedule"
            data-testid="scheduled-row-edit"
            onClick={() => setEditing((current) => !current)}
            size="icon-xs"
            type="button"
            variant="ghost"
          >
            <Pencil aria-hidden />
          </Button>
          <Button
            aria-label="Cancel scheduled message"
            data-testid="scheduled-row-cancel"
            onClick={() => onCancel(message.id)}
            size="icon-xs"
            type="button"
            variant="ghost"
          >
            <Trash2 aria-hidden className="text-destructive" />
          </Button>
        </div>
      </div>
      {editing ? (
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center gap-2">
            <Input
              aria-label="New delivery time"
              className="tabular-nums"
              onChange={(event) => setDatetimeValue(event.target.value)}
              type="datetime-local"
              value={datetimeValue}
            />
            <Button onClick={commitEdit} size="sm" type="button">
              Save
            </Button>
          </div>
          {error != null ? (
            <p className="text-xs text-destructive" role="alert">
              {error}
            </p>
          ) : null}
        </div>
      ) : null}
    </li>
  );
}

export function ScheduledMessagesDialog({
  open,
  onOpenChange,
}: ScheduledMessagesDialogProps) {
  const scheduledQuery = useScheduledMessagesQuery();
  const cancelMutation = useCancelScheduledMessageMutation();
  const scheduleMutation = useScheduleMessageMutation();
  const channelsQuery = useChannelsQuery();
  const channelLabels = React.useMemo(() => {
    const labels = new Map<string, string>();
    for (const channel of channelsQuery.data ?? []) {
      labels.set(channel.id, channel.name);
    }
    return labels;
  }, [channelsQuery.data]);

  const messages = scheduledQuery.data ?? [];

  const handleCancel = React.useCallback(
    (id: string) => {
      cancelMutation.mutate(id, {
        onSuccess: () => toast.success("Scheduled message cancelled"),
        onError: (error) =>
          toast.error(
            error instanceof Error ? error.message : "Failed to cancel.",
          ),
      });
    },
    [cancelMutation],
  );

  const handleSaveEdit = React.useCallback(
    (message: ScheduledMessage, iso: string) => {
      scheduleMutation.mutate(
        {
          channelId: message.channelId,
          content: message.content,
          replyTo: message.replyTo,
          mentions: message.mentions,
          scheduledAt: iso,
        },
        {
          onSuccess: () => {
            toast.success("Scheduled message updated");
            cancelMutation.mutate(message.id, {
              onError: () => {},
            });
          },
          onError: (error) =>
            toast.error(
              error instanceof Error ? error.message : "Failed to reschedule.",
            ),
        },
      );
    },
    [cancelMutation, scheduleMutation],
  );

  const isPending =
    scheduledQuery.isLoading ||
    scheduledQuery.isFetching ||
    channelsQuery.isLoading;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <CalendarClock aria-hidden className="size-4" />
            Scheduled messages
          </DialogTitle>
          <DialogDescription>
            Messages that will be delivered later. Cancel or reschedule any
            pending delivery.
          </DialogDescription>
        </DialogHeader>

        {isPending && messages.length === 0 ? (
          <p className="py-6 text-center text-sm text-muted-foreground">
            Loading scheduled messages…
          </p>
        ) : messages.length === 0 ? (
          <p className="py-6 text-center text-sm text-muted-foreground">
            Nothing scheduled. Use the clock in the composer to queue a message
            for later.
          </p>
        ) : (
          <ul className="max-h-80 space-y-2 overflow-y-auto pr-1">
            {messages.map((message) => (
              <ScheduledMessageRow
                channelLabel={channelLabels.get(message.channelId) ?? null}
                key={message.id}
                message={message}
                onCancel={handleCancel}
                onSaveEdit={handleSaveEdit}
              />
            ))}
          </ul>
        )}

        <div className="flex justify-end">
          <Button
            data-testid="scheduled-view-close"
            onClick={() => onOpenChange(false)}
            type="button"
          >
            Close
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
