import type { SubscribeIntent } from "@/features/meetings/api";
import { SubscribeView } from "@/features/meetings/ui/SubscribeView";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type SubscribeDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Resume an existing pending invoice instead of starting at the plan list. */
  initialIntent?: SubscribeIntent;
  /** Payment settled — the caller retries whatever hit the 402 (e.g. a pending
   * `registerRoom`) and typically closes the dialog. */
  onSettled: () => void;
};

export function SubscribeDialog({
  open,
  onOpenChange,
  initialIntent,
  onSettled,
}: SubscribeDialogProps) {
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        className="max-w-lg"
        data-testid="meeting-subscribe-dialog"
      >
        <DialogHeader>
          <DialogTitle>Set up meeting hosting</DialogTitle>
          <DialogDescription>
            Subscriptions are billed in sats over Lightning. Bring any wallet.
          </DialogDescription>
        </DialogHeader>
        {open ? (
          <SubscribeView
            initialIntent={initialIntent}
            onClose={() => onOpenChange(false)}
            onSettled={onSettled}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
