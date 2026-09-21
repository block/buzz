import {
  CopyPlus,
  EllipsisVertical,
  Pencil,
  Share2,
  Trash2,
} from "lucide-react";

import { useTranslation } from "@/i18n";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

export function PersonaActionsMenu({
  isActionPending,
  isPending,
  persona,
  linkedAgent,
  onDuplicate,
  onEdit,
  onShare,
  onDeactivate,
  onDelete,
}: {
  isActionPending: boolean;
  isPending: boolean;
  persona: AgentPersona;
  /** Profile agent instance linked to this definition, if one exists. */
  linkedAgent: ManagedAgent | undefined;
  onDuplicate: (persona: AgentPersona) => void;
  onEdit: (persona: AgentPersona) => void;
  onShare: (
    persona: AgentPersona,
    linkedAgent: ManagedAgent | undefined,
  ) => void;
  onDeactivate: (persona: AgentPersona) => void;
  onDelete: (persona: AgentPersona) => void;
}) {
  const { t } = useTranslation();
  const disabled = isActionPending || isPending;
  const canEdit = !persona.sourceTeam;

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={t("agents.persona-actions.open-aria", {
            name: persona.displayName,
          })}
          className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          type="button"
        >
          <EllipsisVertical className="h-4 w-4" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        {canEdit ? (
          <DropdownMenuItem disabled={disabled} onClick={() => onEdit(persona)}>
            <Pencil className="h-4 w-4" />
            {t("agents.teams-section.edit")}
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuItem
          disabled={disabled}
          onClick={() => onDuplicate(persona)}
        >
          <CopyPlus className="h-4 w-4" />
          {t("agents.teams-section.duplicate")}
        </DropdownMenuItem>
        <DropdownMenuItem
          disabled={disabled}
          onClick={() => onShare(persona, linkedAgent)}
        >
          <Share2 className="h-4 w-4" />
          {t("agents.teams-section.share")}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        {persona.sourceTeam ? (
          <DropdownMenuItem disabled>
            <Trash2 className="h-4 w-4" />
            {t("agents.persona-actions.managed-by-team")}
          </DropdownMenuItem>
        ) : (
          <DropdownMenuItem
            className="text-destructive focus:text-destructive"
            disabled={disabled}
            onClick={() => {
              if (persona.isBuiltIn) {
                onDeactivate(persona);
                return;
              }

              onDelete(persona);
            }}
          >
            <Trash2 className="h-4 w-4" />
            {t("agents.teams-section.delete")}
          </DropdownMenuItem>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
