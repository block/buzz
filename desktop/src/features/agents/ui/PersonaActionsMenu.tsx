import {
  BookUser,
  CopyPlus,
  Ellipsis,
  FileDown,
  Pencil,
  Trash2,
} from "lucide-react";

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
  onExportSnapshot,
  onDeactivate,
  onDelete,
}: {
  isActionPending: boolean;
  isPending: boolean;
  persona: AgentPersona;
  /** Profile agent instance linked to this definition, if one exists. Used to
   *  supply a memory-source pubkey when the user exports with memory. */
  linkedAgent: ManagedAgent | undefined;
  onDuplicate: (persona: AgentPersona) => void;
  onEdit: (persona: AgentPersona) => void;
  onShare: (persona: AgentPersona) => void;
  onExportSnapshot: (
    persona: AgentPersona,
    linkedAgent: ManagedAgent | undefined,
  ) => void;
  onDeactivate: (persona: AgentPersona) => void;
  onDelete: (persona: AgentPersona) => void;
}) {
  const disabled = isActionPending || isPending;
  const canEdit = !persona.isBuiltIn && !persona.sourceTeam;

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <button
          aria-label={`Open actions for ${persona.displayName}`}
          className="flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          type="button"
        >
          <Ellipsis className="h-4 w-4" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        <DropdownMenuItem disabled={disabled} onClick={() => onShare(persona)}>
          <BookUser className="h-4 w-4" />
          Catalog options
        </DropdownMenuItem>
        {canEdit ? (
          <DropdownMenuItem disabled={disabled} onClick={() => onEdit(persona)}>
            <Pencil className="h-4 w-4" />
            Edit
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuItem
          disabled={disabled}
          onClick={() => onDuplicate(persona)}
        >
          <CopyPlus className="h-4 w-4" />
          Duplicate
        </DropdownMenuItem>
        <DropdownMenuItem
          disabled={disabled}
          onClick={() => onExportSnapshot(persona, linkedAgent)}
        >
          <FileDown className="h-4 w-4" />
          Export snapshot
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        {persona.sourceTeam ? (
          <DropdownMenuItem disabled>
            <Trash2 className="h-4 w-4" />
            Managed by team
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
            Remove from My Agents
          </DropdownMenuItem>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
