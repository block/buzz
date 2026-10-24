// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
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
            {uiT("Remove {0} member{1}?", { p0: count, p1: plural ? "s" : "" })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {uiT(
              "{0} will be removed from this team. Do you also want to remove the {1} completely?",
              { p0: memberNames.join(", "), p1: plural ? "agents" : "agent" },
            )}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <Button
            onClick={() => onOpenChange(false)}
            size="sm"
            type="button"
            variant="outline"
          >
            {uiT("Cancel")}
          </Button>
          <Button
            disabled={isPending}
            onClick={onKeepAgents}
            size="sm"
            type="button"
            variant="outline"
          >
            {uiT("Keep {0}", { p0: plural ? "agents" : "agent" })}
          </Button>
          <Button
            disabled={isPending}
            onClick={onRemoveAgents}
            size="sm"
            type="button"
            variant="destructive"
          >
            {uiT("Remove {0}", { p0: plural ? "agents" : "agent" })}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
