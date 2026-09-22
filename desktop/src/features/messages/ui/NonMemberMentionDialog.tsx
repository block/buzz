import * as React from "react";
import { useTranslation } from "@/i18n";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { PRIVATE_CHANNEL_ADD_DENIED_MESSAGE } from "@/features/channels/lib/channelMemberAdmission";

type NonMemberMentionDialogProps = {
  /** False in a private channel the viewer doesn't own/administer. */
  canInvite: boolean;
  error: string | null;
  isInvitePending: boolean;
  names: string[];
  onDismiss: () => void;
  /** Omit when publication requires the intended recipients to be invited. */
  onDoNothing?: () => void;
  onInvite: () => void;
  open: boolean;
  /** Restore the initiating editor when it still owns the visible draft. */
  onRestoreFocus?: () => void;
};

export function NonMemberMentionDialog({
  canInvite,
  error,
  isInvitePending,
  names,
  onDismiss,
  onDoNothing,
  onInvite,
  open,
  onRestoreFocus,
}: NonMemberMentionDialogProps) {
  const { t } = useTranslation();
  const safeActionRef = React.useRef<HTMLButtonElement>(null);
  const restoreFocusRef = React.useRef(onRestoreFocus);
  return (
    <AlertDialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen) {
          onDismiss();
        }
      }}
      open={open}
    >
      <AlertDialogContent
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          restoreFocusRef.current = onRestoreFocus;
          safeActionRef.current?.focus();
        }}
        onCloseAutoFocus={(event) => {
          if (!restoreFocusRef.current) return;
          event.preventDefault();
          // Pending state/inert is released by the async cancellation continuation.
          // Wait for its React commit; the source checks that it is still visible.
          requestAnimationFrame(() => restoreFocusRef.current?.());
        }}
      >
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t("messages.mention.dialog-title")}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t("messages.mention.nonmember-names", {
              count: names.length,
              names: names.join(", "),
            })}{" "}
            {onDoNothing
              ? canInvite
                ? t("messages.mention.invite-or-send")
                : `${PRIVATE_CHANNEL_ADD_DENIED_MESSAGE} ${t(
                    "messages.mention.denied-still-send",
                  )}`
              : canInvite
                ? t("messages.mention.invite-or-cancel")
                : PRIVATE_CHANNEL_ADD_DENIED_MESSAGE}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {error ? (
          <p
            role="alert"
            className="rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive"
          >
            {error}
          </p>
        ) : null}
        <AlertDialogFooter>
          <Button
            ref={safeActionRef}
            disabled={Boolean(onDoNothing) && isInvitePending}
            onClick={onDoNothing ?? onDismiss}
            size="sm"
            type="button"
            variant="outline"
          >
            {onDoNothing
              ? canInvite
                ? t("messages.mention.do-nothing")
                : t("messages.mention.send-anyway")
              : t("messages.mention.cancel")}
          </Button>
          {canInvite ? (
            <Button
              disabled={isInvitePending}
              onClick={onInvite}
              size="sm"
              type="button"
            >
              {isInvitePending
                ? t("messages.mention.inviting")
                : t("messages.mention.invite")}
            </Button>
          ) : null}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
