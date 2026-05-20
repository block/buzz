import { Headphones } from "lucide-react";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";

/**
 * Pre-join "use headphones" confirmation dialog.
 *
 * Shown once per browser-tab session when the user clicks the start/join
 * huddle button while echo cancellation is missing. The mic isn't opened
 * and `start_huddle` / `join_huddle` is not called until the user clicks
 * "Continue". After a confirmed continue (or an explicit cancel) the gate
 * remembers its decision in `sessionStorage` so the same user isn't
 * nagged on every channel hop.
 *
 * Pair with [`useHeadphonesGate`]. The hook owns the storage + decision
 * state; this component is presentational.
 *
 * Removable in one diff alongside `HeadphonesNotice` when the WebAudio
 * AEC follow-up flips `aecMissing` to false in `HuddleBar`.
 */
export function HeadphonesGate({
  open,
  onContinue,
  onCancel,
}: {
  open: boolean;
  onContinue: () => void;
  onCancel: () => void;
}) {
  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        // Radix calls onOpenChange(false) when the user clicks outside or
        // presses Escape. Treat that as cancel — never as a silent join.
        if (!next) onCancel();
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle className="flex items-center gap-2">
            <Headphones className="h-4 w-4" />
            Headphones recommended
          </AlertDialogTitle>
          <AlertDialogDescription>
            Echo cancellation lands in the next release. Until then, two or more
            people on speakers in the same room will hear themselves echo. Use
            headphones for the best experience.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel onClick={onCancel}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            data-testid="huddle-headphones-gate-continue"
            onClick={onContinue}
          >
            Continue
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
