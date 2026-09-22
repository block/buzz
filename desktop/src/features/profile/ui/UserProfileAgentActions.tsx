import * as React from "react";
import {
  Archive,
  ArchiveRestore,
  CopyPlus,
  Download,
  Power,
  Settings,
} from "lucide-react";

import type { IdentityArchiveActions } from "@/features/identity-archive/hooks";
import { ArchiveConfirmDialog } from "@/features/profile/ui/ArchiveConfirmDialog";
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
  archiveActions,
  isPending,
  isBot = false,
  managedAgent,
  onDuplicatePersona,
  onExportPersona,
  onToggleAutoStart,
  personaActionKey,
}: {
  archiveActions?: IdentityArchiveActions;
  isPending: boolean;
  isBot?: boolean;
  managedAgent?: ManagedAgent;
  onDuplicatePersona?: () => void;
  onExportPersona?: () => void;
  onToggleAutoStart?: () => void;
  personaActionKey?: string;
}) {
  const [archiveConfirmOpen, setArchiveConfirmOpen] = React.useState(false);
  const actionKey = managedAgent?.pubkey ?? "persona-draft";
  const personaKey = personaActionKey ?? actionKey;
  const canToggleAutoStart =
    managedAgent !== undefined &&
    managedAgent.backend.type === "local" &&
    onToggleAutoStart !== undefined;
  const autoStartSwitchId = `user-profile-agent-auto-start-${actionKey}`;
  const hasPrimaryActions = Boolean(onDuplicatePersona || onExportPersona);
  const hasArchiveAction =
    archiveActions?.canArchive === true &&
    archiveActions.isArchived !== undefined;
  const hasActions =
    canToggleAutoStart || hasPrimaryActions || hasArchiveAction;

  if (!hasActions) {
    return null;
  }

  const archiveLabel = isBot ? "Archive agent" : "Archive identity";
  const unarchiveLabel = isBot ? "Unarchive agent" : "Unarchive identity";

  return (
    <>
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
          {hasArchiveAction && (canToggleAutoStart || hasPrimaryActions) ? (
            <DropdownMenuSeparator />
          ) : null}
          {hasArchiveAction && archiveActions ? (
            archiveActions.isArchived ? (
              <DropdownMenuItem
                data-testid="user-profile-unarchive-identity"
                disabled={isPending}
                onClick={archiveActions.unarchive}
              >
                <ArchiveRestore className="h-4 w-4" />
                {archiveActions.isPending ? "Unarchiving…" : unarchiveLabel}
              </DropdownMenuItem>
            ) : (
              <DropdownMenuItem
                data-testid="user-profile-archive-identity"
                disabled={isPending}
                onSelect={() => setArchiveConfirmOpen(true)}
              >
                <Archive className="h-4 w-4" />
                {archiveActions.isPending ? "Archiving…" : archiveLabel}
              </DropdownMenuItem>
            )
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
      {hasArchiveAction && archiveActions ? (
        <ArchiveConfirmDialog
          isBot={isBot}
          isPending={archiveActions.isPending}
          onConfirm={() => {
            archiveActions.archive();
            setArchiveConfirmOpen(false);
          }}
          onOpenChange={setArchiveConfirmOpen}
          open={archiveConfirmOpen}
        />
      ) : null}
    </>
  );
}

export function UserProfileAgentSettingsMenuSlot({
  archiveActions,
  canInstantiateAgent,
  canManagePersona,
  isAgentActionPending,
  isBot,
  managedAgent,
  onDuplicatePersona,
  onExportPersona,
  onToggleAutoStart,
  personaActionKey,
  viewerIsOwner,
}: {
  archiveActions: IdentityArchiveActions;
  canInstantiateAgent: boolean;
  canManagePersona: boolean;
  isAgentActionPending: boolean;
  isBot: boolean;
  managedAgent?: ManagedAgent;
  onDuplicatePersona: () => void;
  onExportPersona: () => void;
  onToggleAutoStart: () => void;
  personaActionKey?: string;
  viewerIsOwner: boolean;
}) {
  const canShowArchiveAction =
    archiveActions.canArchive && archiveActions.isArchived !== undefined;
  const settingsActionPending =
    isAgentActionPending || archiveActions.isPending;
  const sharedProps = {
    archiveActions: !isBot && canShowArchiveAction ? archiveActions : undefined,
    isBot,
    isPending: settingsActionPending,
    onDuplicatePersona:
      !isBot && canManagePersona ? onDuplicatePersona : undefined,
    onExportPersona: !isBot && canManagePersona ? onExportPersona : undefined,
    personaActionKey,
  };

  if (viewerIsOwner && managedAgent) {
    return (
      <UserProfileAgentSettingsMenu
        {...sharedProps}
        managedAgent={managedAgent}
        onToggleAutoStart={onToggleAutoStart}
      />
    );
  }

  if (canInstantiateAgent) {
    return <UserProfileAgentSettingsMenu {...sharedProps} />;
  }

  if (canShowArchiveAction && !isBot) {
    return (
      <UserProfileAgentSettingsMenu
        archiveActions={archiveActions}
        isBot={isBot}
        isPending={settingsActionPending}
      />
    );
  }

  return null;
}
