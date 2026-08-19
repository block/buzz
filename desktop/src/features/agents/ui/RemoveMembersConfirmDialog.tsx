import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

type RemoveMembersConfirmDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  isPending: boolean;
  memberNames: string[];
  onKeepAgents: () => void;
  onRemoveAgents: () => void;
};

export function RemoveMembersConfirmDialog({
  open,
  onOpenChange,
  isPending,
  memberNames,
  onKeepAgents,
  onRemoveAgents,
}: RemoveMembersConfirmDialogProps) {
  const count = memberNames.length;
  const plural = count !== 1;

  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            Remove {count} member{plural ? "s" : ""}?
          </AlertDialogTitle>
          <AlertDialogDescription>
            {memberNames.join(", ")} will be removed from this team. Do you also
            want to remove the {plural ? "agents" : "agent"} completely?
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <Button
            onClick={() => onOpenChange(false)}
            size="sm"
            type="button"
            variant="outline"
          >
            Cancel
          </Button>
          <Button
            disabled={isPending}
            onClick={onKeepAgents}
            size="sm"
            type="button"
            variant="outline"
          >
            Keep {plural ? "agents" : "agent"}
          </Button>
          <Button
            disabled={isPending}
            onClick={onRemoveAgents}
            size="sm"
            type="button"
            variant="destructive"
          >
            Remove {plural ? "agents" : "agent"}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
