import { useTranslation } from "@/i18n";
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
  const { t } = useTranslation();
  const count = memberNames.length;

  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t("agents.remove-members.title", { count })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t("agents.remove-members.description", {
              count,
              memberNames: memberNames.join(", "),
            })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <Button
            onClick={() => onOpenChange(false)}
            size="sm"
            type="button"
            variant="outline"
          >
            {t("agents.remove-members.cancel")}
          </Button>
          <Button
            disabled={isPending}
            onClick={onKeepAgents}
            size="sm"
            type="button"
            variant="outline"
          >
            {t("agents.remove-members.keep-agent", { count })}
          </Button>
          <Button
            disabled={isPending}
            onClick={onRemoveAgents}
            size="sm"
            type="button"
            variant="destructive"
          >
            {t("agents.remove-members.remove-agent", { count })}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
