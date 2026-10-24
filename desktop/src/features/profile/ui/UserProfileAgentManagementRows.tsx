// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT, useUiLanguage } from "@/shared/i18n/uiTranslation";
import * as React from "react";
import {
  Archive,
  ArchiveRestore,
  CopyPlus,
  Download,
  Sparkles,
  Trash2,
  type LucideIcon,
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
import { PanelSectionGroup } from "@/shared/ui/PanelSectionGroup";

export function UserProfileAgentManagementRows({
  archiveActions,
  canArchiveAgent,
  canDeleteAgent,
  isDeletePending,
  managedAgent,
  supplementalAction,
  onCreateCard,
  onDeleteAgent,
  onDuplicateAgent,
  onExportAgent,
}: {
  archiveActions: IdentityArchiveActions;
  canArchiveAgent: boolean;
  canDeleteAgent: boolean;
  isDeletePending: boolean;
  managedAgent?: ManagedAgent;
  supplementalAction?: React.ReactNode;
  /** Mint an agent trading card. Present only for owner-managed personas. */
  onCreateCard?: () => void;
  onDeleteAgent: () => void;
  onDuplicateAgent?: () => void;
  onExportAgent?: () => void;
}) {
  if (
    !onCreateCard &&
    !onDuplicateAgent &&
    !onExportAgent &&
    !supplementalAction &&
    !canArchiveAgent &&
    !canDeleteAgent
  ) {
    return null;
  }

  return (
    <PanelSectionGroup testId="user-profile-agent-management-section">
      {onDuplicateAgent ? (
        <ProfileAgentActionRow
          disabled={isDeletePending}
          icon={CopyPlus}
          label={uiT("Duplicate agent")}
          onClick={onDuplicateAgent}
          testId="user-profile-duplicate-agent-row"
        />
      ) : null}
      {onExportAgent ? (
        <ProfileAgentActionRow
          disabled={isDeletePending}
          icon={Download}
          label={uiT("Export agent")}
          onClick={onExportAgent}
          testId="user-profile-export-agent-row"
        />
      ) : null}
      {onCreateCard ? (
        <ProfileAgentActionRow
          disabled={isDeletePending}
          icon={Sparkles}
          label={uiT("Create trading card")}
          onClick={onCreateCard}
          testId="user-profile-create-card-row"
        />
      ) : null}
      {supplementalAction}
      {canArchiveAgent ? (
        <ProfileArchiveAgentRow archiveActions={archiveActions} />
      ) : null}
      {canDeleteAgent ? (
        <ProfileDeleteAgentRow
          isPending={isDeletePending}
          managedAgent={managedAgent}
          onDelete={onDeleteAgent}
        />
      ) : null}
    </PanelSectionGroup>
  );
}

export function ProfileAgentActionRow({
  destructive = false,
  disabled = false,
  icon: Icon,
  iconClassName,
  label,
  onClick,
  testId,
}: {
  destructive?: boolean;
  disabled?: boolean;
  icon: LucideIcon;
  iconClassName?: string;
  label: string;
  onClick: () => void;
  testId: string;
}) {
  return (
    <button
      className="flex min-h-16 w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40 disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      <Icon
        className={
          iconClassName ??
          (destructive
            ? "h-4 w-4 shrink-0 text-destructive"
            : "h-4 w-4 shrink-0 text-muted-foreground")
        }
        data-slot="profile-action-icon"
      />
      <span
        className={
          destructive
            ? "min-w-0 flex-1 text-sm font-medium text-destructive"
            : "min-w-0 flex-1 text-sm font-medium"
        }
      >
        {label}
      </span>
    </button>
  );
}

function ProfileArchiveAgentRow({
  archiveActions,
}: {
  archiveActions: IdentityArchiveActions;
}) {
  useUiLanguage();

  const [confirmOpen, setConfirmOpen] = React.useState(false);
  const isArchived = archiveActions.isArchived === true;
  const Icon = isArchived ? ArchiveRestore : Archive;
  const label = archiveActions.isPending
    ? isArchived
      ? uiT("Unarchiving…")
      : uiT("Archiving…")
    : isArchived
      ? uiT("Unarchive agent")
      : uiT("Archive agent");

  return (
    <>
      <ProfileAgentActionRow
        disabled={archiveActions.isPending}
        icon={Icon}
        label={label}
        onClick={() => {
          if (isArchived) {
            archiveActions.unarchive();
            return;
          }
          setConfirmOpen(true);
        }}
        testId={
          isArchived
            ? "user-profile-unarchive-agent-row"
            : "user-profile-archive-agent-row"
        }
      />
      <ArchiveConfirmDialog
        isBot
        isPending={archiveActions.isPending}
        onConfirm={() => {
          archiveActions.archive();
          setConfirmOpen(false);
        }}
        onOpenChange={setConfirmOpen}
        open={confirmOpen}
      />
    </>
  );
}

function ProfileDeleteAgentRow({
  isPending,
  managedAgent,
  onDelete,
}: {
  isPending: boolean;
  managedAgent?: ManagedAgent;
  onDelete: () => void;
}) {
  useUiLanguage();

  const [confirmOpen, setConfirmOpen] = React.useState(false);

  return (
    <>
      <ProfileAgentActionRow
        destructive
        disabled={isPending}
        icon={Trash2}
        label={uiT("Delete agent")}
        onClick={() => {
          if (managedAgent) {
            setConfirmOpen(true);
            return;
          }
          onDelete();
        }}
        testId="user-profile-delete-agent-row"
      />
      {managedAgent ? (
        <AgentDeleteConfirmDialog
          agent={managedAgent}
          isPending={isPending}
          onConfirm={() => {
            setConfirmOpen(false);
            onDelete();
          }}
          onOpenChange={setConfirmOpen}
          open={confirmOpen}
        />
      ) : null}
    </>
  );
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
          <AlertDialogTitle>{uiT("Delete this agent?")}</AlertDialogTitle>
          <AlertDialogDescription>
            {isProviderAgent
              ? uiT(
                  "Deleting removes this agent’s local management record, not its remote deployment.",
                )
              : uiT(
                  "Deleting this agent stops and removes the agent from this community.",
                )}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <ul className="list-disc space-y-1.5 pl-5 text-sm text-muted-foreground">
          <li>
            {uiT("Removes the local management record and saved agent key")}
          </li>
          <li>{uiT("Removes the agent from every channel it belongs to")}</li>
          <li>
            {uiT(
              "Archives the agent's identity on the relay so it no longer appears in member lists or mention suggestions",
            )}
          </li>
          <li>
            {isProviderAgent
              ? uiT(
                  "Unless the agent is known to be Offline, Buzz first requests shutdown through a channel when available. A failed request cancels deletion. The remote process may still be running even after a successful request.",
                )
              : uiT("Stops any local agent process before deleting the record")}
          </li>
        </ul>
        <p className="text-sm text-muted-foreground">
          {uiT(
            "Archive this agent if you want to hide it instead of removing it.",
          )}
        </p>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button type="button" variant="outline">
              {uiT("Cancel")}
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction
            className={buttonVariants({ variant: "destructive" })}
            data-testid="agent-delete-confirm-action"
            disabled={isPending}
            onClick={onConfirm}
          >
            {isPending ? uiT("Deleting…") : uiT("Delete agent")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
