import type { DeleteTarget } from "@/features/agents/agentManagement";
import type { AgentPersona } from "@/shared/api/types";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { PersonaDeleteDialog } from "./PersonaDeleteDialog";

type AgentDeleteRequestDialogProps = {
  target: DeleteTarget;
  instanceCount: number;
  error: string | null;
  isPending: boolean;
  onConfirm: (persona: AgentPersona) => void;
  onClose: () => void;
};

/** Owner confirmation for an agent-requested delete; nothing is deleted until the owner clicks Delete. */
export function AgentDeleteRequestDialog({
  target,
  instanceCount,
  error,
  isPending,
  onConfirm,
  onClose,
}: AgentDeleteRequestDialogProps) {
  if (target.status === "loading") return null;

  if (target.status === "error") {
    return (
      <AlertDialog
        onOpenChange={(open) => {
          if (!open) onClose();
        }}
        open
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Can't delete agent</AlertDialogTitle>
            <AlertDialogDescription>{target.message}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogAction asChild>
              <Button type="button">OK</Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    );
  }

  return (
    <PersonaDeleteDialog
      error={error}
      instanceCount={instanceCount}
      isPending={isPending}
      keepOpenOnConfirm
      onConfirm={onConfirm}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      open
      persona={target.persona}
    />
  );
}
