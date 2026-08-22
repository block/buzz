import type { RegisteredAgentReference } from "@/shared/api/tauriRegisteredAgents";
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

export function RemoveRegisteredAgentDialog({
  isPending,
  onConfirm,
  onOpenChange,
  reference,
}: {
  isPending: boolean;
  onConfirm: (reference: RegisteredAgentReference) => void;
  onOpenChange: (open: boolean) => void;
  reference: RegisteredAgentReference | null;
}) {
  const label = reference?.label?.trim() || reference?.pubkey || "this agent";
  return (
    <AlertDialog onOpenChange={onOpenChange} open={reference !== null}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Remove existing agent reference?</AlertDialogTitle>
          <AlertDialogDescription>
            Remove the reference to {label}? This removes only the local card
            and reference. It does not delete the identity, stop a responder,
            remove channel membership, or erase messages.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={isPending}>Cancel</AlertDialogCancel>
          <AlertDialogAction
            className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            disabled={isPending}
            onClick={() => {
              if (reference) onConfirm(reference);
            }}
          >
            Remove reference
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
