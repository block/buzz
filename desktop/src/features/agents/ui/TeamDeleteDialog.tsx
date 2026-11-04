// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import type { AgentTeam } from "@/shared/api/types";
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
import { Button } from "@/shared/ui/button";

type TeamDeleteDialogProps = {
  open: boolean;
  team: AgentTeam | null;
  onConfirm: (team: AgentTeam) => void;
  onOpenChange: (open: boolean) => void;
};

export function TeamDeleteDialog({
  open,
  team,
  onConfirm,
  onOpenChange,
}: TeamDeleteDialogProps) {
  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{uiT("Delete team?")}</AlertDialogTitle>
          <AlertDialogDescription>
            {team
              ? uiT(
                  'Delete "{0}". Already-deployed agents are not affected, but this team template will no longer be available.',
                  { p0: team.name },
                )
              : uiT("Delete this team.")}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button type="button" variant="outline">
              {uiT("Cancel")}
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction asChild>
            <Button
              onClick={() => {
                if (team) {
                  onConfirm(team);
                }
              }}
              type="button"
              variant="destructive"
            >
              {uiT("Delete")}
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
