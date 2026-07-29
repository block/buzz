import type { NotifyMode } from "@/features/messages/lib/channelNotify";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

type ChannelNotifyDialogProps = {
  isSendPending: boolean;
  /** Channel member count, used to size the `@channel` prompt. */
  memberCount: number;
  mode: NotifyMode | null;
  onCancel: () => void;
  onConfirm: () => void;
};

/**
 * Confirmation shown before a message that carries `@channel` or `@here` is
 * sent. Mirrors the non-member mention prompt's seam in the composer.
 */
export function ChannelNotifyDialog({
  isSendPending,
  memberCount,
  mode,
  onCancel,
  onConfirm,
}: ChannelNotifyDialogProps) {
  const isChannel = mode === "channel";

  return (
    <AlertDialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen) {
          onCancel();
        }
      }}
      open={mode !== null}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {isChannel
              ? memberCount > 0
                ? `Notify all ${memberCount} members?`
                : "Notify everyone in this channel?"
              : "Notify members who are online?"}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {isChannel
              ? "@channel notifies every member of this channel, even when they are away. Members who muted the channel are not notified."
              : "@here notifies only the members who are online right now. Members who muted the channel are not notified."}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <Button
            disabled={isSendPending}
            onClick={onCancel}
            size="sm"
            type="button"
            variant="outline"
          >
            Cancel
          </Button>
          <Button
            data-testid="channel-notify-confirm"
            disabled={isSendPending}
            onClick={onConfirm}
            size="sm"
            type="button"
          >
            {isChannel ? "Notify channel" : "Notify online members"}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
