import {
  Copy,
  MoreHorizontal,
  Pencil,
  Play,
  Power,
  PowerOff,
  Trash2,
} from "lucide-react";

import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

type WorkflowActionsMenuProps = {
  isEnabled: boolean;
  isTogglingEnabled?: boolean;
  onDelete: () => void;
  onDuplicate: () => void;
  onEdit: () => void;
  onToggleEnabled: () => void;
  onTrigger: () => void;
};

export function WorkflowActionsMenu({
  isEnabled,
  isTogglingEnabled = false,
  onDelete,
  onDuplicate,
  onEdit,
  onToggleEnabled,
  onTrigger,
}: WorkflowActionsMenuProps) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          aria-label="Workflow actions"
          className="h-8 w-8 text-muted-foreground hover:bg-background/80 hover:text-foreground data-[state=open]:bg-background/80 data-[state=open]:text-foreground"
          size="icon"
          type="button"
          variant="ghost"
        >
          <MoreHorizontal className="h-4 w-4" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={onTrigger}>
          <Play className="mr-2 h-4 w-4" />
          Trigger
        </DropdownMenuItem>
        <DropdownMenuItem onClick={onEdit}>
          <Pencil className="mr-2 h-4 w-4" />
          Edit
        </DropdownMenuItem>
        <DropdownMenuItem onClick={onDuplicate}>
          <Copy className="mr-2 h-4 w-4" />
          Duplicate
        </DropdownMenuItem>
        <DropdownMenuCheckboxItem
          checked={isEnabled}
          className="gap-2 pl-2 [&>span:first-child]:hidden"
          disabled={isTogglingEnabled}
          onCheckedChange={(checked) => {
            if (checked !== isEnabled) onToggleEnabled();
          }}
          onSelect={(event) => event.preventDefault()}
        >
          {isEnabled ? (
            <Power className="mr-2 h-4 w-4 shrink-0" />
          ) : (
            <PowerOff className="mr-2 h-4 w-4 shrink-0" />
          )}
          <span>Enable</span>
          <span
            aria-hidden="true"
            className={
              "ml-auto inline-flex h-5 w-9 shrink-0 items-center rounded-full border-2 border-transparent transition-colors " +
              (isEnabled ? "bg-primary" : "bg-input")
            }
            data-testid="workflow-enabled-switch-visual"
          >
            <span
              className={
                "block h-4 w-4 rounded-full bg-background transition-transform " +
                (isEnabled ? "translate-x-4" : "translate-x-0")
              }
            />
          </span>
        </DropdownMenuCheckboxItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem className="text-destructive" onClick={onDelete}>
          <Trash2 className="mr-2 h-4 w-4" />
          Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
