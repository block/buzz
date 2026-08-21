import * as React from "react";

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

type FibreDismissReasonDialogProps = {
  /** Title of the fibre being explained, or null when nothing is pending. */
  fibreTitle: string | null;
  onCancel: () => void;
  onSubmit: (note: string) => void;
};

/**
 * Optional follow-up to a dismissal. The fibre is already gone by the time
 * this opens — the note only teaches triage why, so cancelling costs nothing.
 */
export function FibreDismissReasonDialog({
  fibreTitle,
  onCancel,
  onSubmit,
}: FibreDismissReasonDialogProps) {
  const [note, setNote] = React.useState("");

  React.useEffect(() => {
    if (fibreTitle !== null) setNote("");
  }, [fibreTitle]);

  const submit = () => {
    const trimmed = note.trim();
    if (!trimmed) {
      onCancel();
      return;
    }
    onSubmit(trimmed);
  };

  return (
    <Dialog
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
      open={fibreTitle !== null}
    >
      <DialogContent className="sm:max-w-md" data-testid="fibre-reason-dialog">
        <DialogHeader>
          <DialogTitle>Why was this not worth your attention?</DialogTitle>
          <DialogDescription>
            Triage keeps this as a standing instruction, so similar fibres rank
            lower or stop appearing.
          </DialogDescription>
        </DialogHeader>
        {fibreTitle ? (
          <p className="truncate text-sm text-muted-foreground">{fibreTitle}</p>
        ) : null}
        <Textarea
          autoFocus
          data-testid="fibre-reason-input"
          onChange={(event) => setNote(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
              event.preventDefault();
              submit();
            }
          }}
          placeholder="Social channel, I never act on these."
          value={note}
        />
        <DialogFooter>
          <Button onClick={onCancel} type="button" variant="ghost">
            Skip
          </Button>
          <Button
            data-testid="fibre-reason-save"
            disabled={note.trim().length === 0}
            onClick={submit}
            type="button"
          >
            Save reason
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
