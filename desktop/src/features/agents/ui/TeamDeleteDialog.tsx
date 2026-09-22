import { useTranslation } from "@/i18n";
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
  const { t } = useTranslation();
  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("agents.team-delete.title")}</AlertDialogTitle>
          <AlertDialogDescription>
            {team
              ? t("agents.team-delete.confirm-name", { name: team.name })
              : t("agents.team-delete.confirm-no-team")}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button type="button" variant="outline">
              {t("agents.team-dialog.cancel")}
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
              {t("agents.teams-section.delete")}
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
