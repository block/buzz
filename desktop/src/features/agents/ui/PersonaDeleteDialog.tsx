import { i18n, useTranslation } from "@/i18n";
import type { AgentPersona } from "@/shared/api/types";
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

type PersonaDeleteDialogProps = {
  open: boolean;
  persona: AgentPersona | null;
  /** Number of managed-agent instances backed by this persona. Omit or pass 0 to suppress the instance-count sentence. */
  instanceCount?: number;
  onConfirm: (persona: AgentPersona) => void;
  onOpenChange: (open: boolean) => void;
};

/**
 * Confirmation copy for deleting a persona. Pure so the cascade archival
 * disclosure stays unit-testable without a renderer: whenever instances are
 * cascade-deleted, each one's identity is also archived on the relay
 * (NIP-IA), and that durable side effect must be disclosed before the
 * destructive confirm — matching the direct agent-delete dialog.
 */
export function personaDeleteDescription(
  persona: AgentPersona | null,
  instanceCount: number,
): string {
  if (!persona) {
    return i18n.t("agents.persona-delete.confirm-no-persona");
  }
  if (instanceCount === 0) {
    return i18n.t("agents.persona-delete.confirm-name", {
      name: persona.displayName,
    });
  }
  const cascade = i18n.t("agents.persona-delete.cascade", {
    count: instanceCount,
  });
  return `${i18n.t("agents.persona-delete.confirm-name", {
    name: persona.displayName,
  })} ${cascade}`;
}

export function PersonaDeleteDialog({
  open,
  persona,
  instanceCount = 0,
  onConfirm,
  onOpenChange,
}: PersonaDeleteDialogProps) {
  const { t } = useTranslation();
  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t("agents.persona-delete.title")}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {personaDeleteDescription(persona, instanceCount)}
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
                if (persona) {
                  onConfirm(persona);
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
