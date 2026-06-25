import { CopyPlus, Download, Power, Settings, Trash2 } from "lucide-react";

import type { ManagedAgent } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Switch } from "@/shared/ui/switch";

export function UserProfileAgentSettingsMenu({
  isPending,
  managedAgent,
  onDelete,
  onDuplicatePersona,
  onExportPersona,
  onToggleAutoStart,
  personaActionKey,
}: {
  isPending: boolean;
  managedAgent?: ManagedAgent;
  onDelete?: () => void;
  onDuplicatePersona?: () => void;
  onExportPersona?: () => void;
  onToggleAutoStart?: () => void;
  personaActionKey?: string;
}) {
  const actionKey = managedAgent?.pubkey ?? "persona-draft";
  const personaKey = personaActionKey ?? actionKey;
  const canToggleAutoStart =
    managedAgent !== undefined &&
    managedAgent.backend.type === "local" &&
    onToggleAutoStart !== undefined;
  const autoStartSwitchId = `user-profile-agent-auto-start-${actionKey}`;
  const hasPrimaryActions = Boolean(onDuplicatePersona || onExportPersona);
  const hasActions =
    canToggleAutoStart || hasPrimaryActions || Boolean(onDelete);

  if (!hasActions) {
    return null;
  }

  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          aria-label="Open profile settings"
          data-testid="user-profile-settings-menu-trigger"
          size="icon"
          type="button"
          variant="ghost"
        >
          <Settings />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="min-w-56"
        onCloseAutoFocus={(event) => event.preventDefault()}
      >
        {canToggleAutoStart ? (
          <DropdownMenuItem
            className="gap-3 pr-2"
            disabled={isPending}
            onSelect={(event) => {
              event.preventDefault();
              onToggleAutoStart();
            }}
          >
            <Power className="h-4 w-4 text-muted-foreground" />
            <span className="min-w-0 flex-1 text-sm font-medium">
              Auto-start
            </span>
            <Switch
              aria-label="Auto-start"
              checked={managedAgent.startOnAppLaunch}
              data-testid={autoStartSwitchId}
              disabled={isPending}
              id={autoStartSwitchId}
              onCheckedChange={onToggleAutoStart}
              onClick={(event) => event.stopPropagation()}
            />
          </DropdownMenuItem>
        ) : null}
        {onDuplicatePersona ? (
          <DropdownMenuItem
            data-testid={`user-profile-persona-duplicate-${personaKey}`}
            disabled={isPending}
            onClick={onDuplicatePersona}
          >
            <CopyPlus className="h-4 w-4" />
            Duplicate
          </DropdownMenuItem>
        ) : null}
        {onExportPersona ? (
          <DropdownMenuItem
            data-testid={`user-profile-persona-export-${personaKey}`}
            disabled={isPending}
            onClick={onExportPersona}
          >
            <Download className="h-4 w-4" />
            Export
          </DropdownMenuItem>
        ) : null}
        {onDelete && (canToggleAutoStart || hasPrimaryActions) ? (
          <DropdownMenuSeparator />
        ) : null}
        {onDelete ? (
          <DropdownMenuItem
            className="text-destructive focus:text-destructive"
            data-testid={`user-profile-agent-delete-${actionKey}`}
            disabled={isPending}
            onClick={onDelete}
          >
            <Trash2 className="h-4 w-4" />
            Delete agent
          </DropdownMenuItem>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
