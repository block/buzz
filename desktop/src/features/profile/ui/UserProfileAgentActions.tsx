import * as React from "react";
import {
  Archive,
  ArchiveRestore,
  CopyPlus,
  Download,
  Power,
  Settings,
  Trash2,
} from "lucide-react";

import type { IdentityArchiveActions } from "@/features/identity-archive/hooks";
import { ArchiveConfirmDialog } from "@/features/profile/ui/ArchiveConfirmDialog";
import type { ManagedAgent } from "@/shared/api/types";
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
import { Button, buttonVariants } from "@/shared/ui/button";
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
  onDelete,
  onDuplicatePersona,
  onExportPersona,
  onToggleAutoStart,
  personaActionKey,
}: {
  archiveActions?: IdentityArchiveActions;
  isPending: boolean;
  isBot?: boolean;
  managedAgent?: ManagedAgent;
  onDelete?: () => void;
  onDuplicatePersona?: () => void;
  onExportPersona?: () => void;
  onToggleAutoStart?: () => void;
  personaActionKey?: string;
}) {
  const [archiveConfirmOpen, setArchiveConfirmOpen] = React.useState(false);
  const [deleteConfirmOpen, setDeleteConfirmOpen] = React.useState(false);
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
  const shouldConfirmAgentDelete =
    managedAgent !== undefined && onDelete !== undefined;
  const hasManageActions = hasArchiveAction || Boolean(onDelete);
  const hasActions =
    canToggleAutoStart || hasPrimaryActions || hasManageActions;

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
          {hasManageActions && (canToggleAutoStart || hasPrimaryActions) ? (
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
          {onDelete && hasArchiveAction ? <DropdownMenuSeparator /> : null}
          {onDelete ? (
            <DropdownMenuItem
              className="text-destructive focus:text-destructive"
              data-testid={`user-profile-agent-delete-${actionKey}`}
              disabled={isPending}
              onSelect={() => {
                if (shouldConfirmAgentDelete) {
                  setDeleteConfirmOpen(true);
                  return;
                }
                onDelete();
              }}
            >
              <Trash2 className="h-4 w-4" />
              Delete agent
            </DropdownMenuItem>
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
      {shouldConfirmAgentDelete ? (
        <AgentDeleteConfirmDialog
          agent={managedAgent}
          isPending={isPending}
          onConfirm={() => {
            setDeleteConfirmOpen(false);
            onDelete();
          }}
          onOpenChange={setDeleteConfirmOpen}
          open={deleteConfirmOpen}
        />
      ) : null}
    </>
  );
}

export function UserProfileAgentSettingsMenuSlot({
  archiveActions,
  canDeletePersona,
  canInstantiateAgent,
  canManagePersona,
  isAgentActionPending,
  isBot,
  managedAgent,
  onDeleteAgent,
  onDeletePersona,
  onDuplicatePersona,
  onExportPersona,
  onToggleAutoStart,
  personaActionKey,
  viewerIsOwner,
}: {
  archiveActions: IdentityArchiveActions;
  canDeletePersona: boolean;
  canInstantiateAgent: boolean;
  canManagePersona: boolean;
  isAgentActionPending: boolean;
  isBot: boolean;
  managedAgent?: ManagedAgent;
  onDeleteAgent: () => void;
  onDeletePersona: () => void;
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
    archiveActions: canShowArchiveAction ? archiveActions : undefined,
    isBot,
    isPending: settingsActionPending,
    onDuplicatePersona: canManagePersona ? onDuplicatePersona : undefined,
    onExportPersona: canManagePersona ? onExportPersona : undefined,
    personaActionKey,
  };

  if (viewerIsOwner && managedAgent) {
    return (
      <UserProfileAgentSettingsMenu
        {...sharedProps}
        managedAgent={managedAgent}
        onDelete={onDeleteAgent}
        onToggleAutoStart={onToggleAutoStart}
      />
    );
  }

  if (canInstantiateAgent) {
    return (
      <UserProfileAgentSettingsMenu
        {...sharedProps}
        onDelete={canDeletePersona ? onDeletePersona : undefined}
      />
    );
  }

  if (canShowArchiveAction) {
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

function AgentDeleteConfirmDialog({
  agent,
  isPending,
  onConfirm,
  onOpenChange,
  open,
}: {
  agent: ManagedAgent;
  isPending: boolean;
  onConfirm: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const isProviderAgent = agent.backend.type === "provider";

  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent data-testid="agent-delete-confirm-dialog">
        <AlertDialogHeader>
          <AlertDialogTitle>Delete this agent?</AlertDialogTitle>
          <AlertDialogDescription>
            Deleting this agent stops and removes the agent from this community.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <ul className="list-disc space-y-1.5 pl-5 text-sm text-muted-foreground">
          <li>Removes the local management record and saved agent key</li>
          <li>Removes the agent from every channel it belongs to</li>
          <li>
            Archives the agent&apos;s identity on the relay so it no longer
            appears in member lists or mention suggestions
          </li>
          <li>
            {isProviderAgent
              ? "Requests remote deletion; if it is online, Buzz first sends a shutdown command when possible. If the deployment cannot be reached through a channel, the remote process may keep running without local management."
              : "Stops any local agent process before deleting the record"}
          </li>
        </ul>
        <p className="text-sm text-muted-foreground">
          You can also archive this agent from the profile settings menu if you
          want to hide the agent instead of removing it.
        </p>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button type="button" variant="outline">
              Cancel
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction
            className={buttonVariants({ variant: "destructive" })}
            data-testid="agent-delete-confirm-action"
            disabled={isPending}
            onClick={onConfirm}
          >
            {isPending ? "Deleting..." : "Delete agent"}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
