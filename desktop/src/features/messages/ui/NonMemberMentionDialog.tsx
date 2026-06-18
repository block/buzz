import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

type NonMemberMentionDialogProps = {
  error: string | null;
  isInvitePending: boolean;
  names: string[];
  onDismiss: () => void;
  onDoNothing: () => void;
  onInvite: () => void;
  open: boolean;
};

export function NonMemberMentionDialog({
  error,
  isInvitePending,
  names,
  onDismiss,
  onDoNothing,
  onInvite,
  open,
}: NonMemberMentionDialogProps) {
  return (
    <AlertDialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen) {
          onDismiss();
        }
      }}
      open={open}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            Mention people outside this channel?
          </AlertDialogTitle>
          <AlertDialogDescription>
            {names.join(", ")} {names.length === 1 ? "is" : "are"} not in this
            channel. Invite them to the channel, or send without inviting them.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {error ? (
          <p className="rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {error}
          </p>
        ) : null}
        <AlertDialogFooter>
          <Button
            disabled={isInvitePending}
            onClick={onDoNothing}
            size="sm"
            type="button"
            variant="outline"
          >
            Do nothing
          </Button>
          <Button
            disabled={isInvitePending}
            onClick={onInvite}
            size="sm"
            type="button"
          >
            {isInvitePending ? "Inviting..." : "Invite"}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
