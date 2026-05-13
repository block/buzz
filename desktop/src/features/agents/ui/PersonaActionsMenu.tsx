import { CopyPlus, Download, Ellipsis, Pencil, Trash2 } from "lucide-react";

import type { AgentPersona } from "@/shared/api/types";
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
  onDuplicate,
  onEdit,
  onExport,
  onDeactivate,
  onDelete,
}: {
  isActionPending: boolean;
  isPending: boolean;
  persona: AgentPersona;
  onDuplicate: (persona: AgentPersona) => void;
  onEdit: (persona: AgentPersona) => void;
  onExport: (persona: AgentPersona) => void;
  onDeactivate: (persona: AgentPersona) => void;
  onDelete: (persona: AgentPersona) => void;
}) {
  const disabled = isActionPending || isPending;

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
        {!persona.isBuiltIn ? (
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
        <DropdownMenuItem disabled={disabled} onClick={() => onExport(persona)}>
          <Download className="h-4 w-4" />
          Export
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        {persona.isBuiltIn ? (
          <DropdownMenuItem
            className="text-destructive focus:text-destructive"
            disabled={disabled}
            onClick={() => onDeactivate(persona)}
          >
            <Trash2 className="h-4 w-4" />
            Remove from My Agents
          </DropdownMenuItem>
        ) : persona.sourcePack ? (
          <DropdownMenuItem disabled>
            <Trash2 className="h-4 w-4" />
            Managed by pack
          </DropdownMenuItem>
        ) : (
          <DropdownMenuItem
            className="text-destructive focus:text-destructive"
            disabled={disabled}
            onClick={() => onDelete(persona)}
          >
            <Trash2 className="h-4 w-4" />
            Delete
          </DropdownMenuItem>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
