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
import { useAgentMemoryQuery } from "@/features/agent-memory/hooks";
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
  channelCount,
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
  channelCount: number;
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
          label="Duplicate agent"
          onClick={onDuplicateAgent}
          testId="user-profile-duplicate-agent-row"
        />
      ) : null}
      {onExportAgent ? (
        <ProfileAgentActionRow
          disabled={isDeletePending}
          icon={Download}
          label="Export agent"
          onClick={onExportAgent}
          testId="user-profile-export-agent-row"
        />
      ) : null}
      {onCreateCard ? (
        <ProfileAgentActionRow
          disabled={isDeletePending}
          icon={Sparkles}
          label="Create trading card"
          onClick={onCreateCard}
          testId="user-profile-create-card-row"
        />
      ) : null}
      {canArchiveAgent ? (
        <ProfileArchiveAgentRow archiveActions={archiveActions} />
      ) : null}
      {canDeleteAgent ? (
        <ProfileDeleteAgentRow
          channelCount={channelCount}
          isPending={isDeletePending}
          managedAgent={managedAgent}
          onDelete={onDeleteAgent}
          onExport={onExportAgent}
        />
      ) : null}
    </PanelSectionGroup>
  );
}

function ProfileAgentActionRow({
  destructive = false,
  disabled = false,
  icon: Icon,
  label,
  onClick,
  testId,
}: {
  destructive?: boolean;
  disabled?: boolean;
  icon: LucideIcon;
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
          destructive
            ? "h-4 w-4 shrink-0 text-destructive"
            : "h-4 w-4 shrink-0 text-muted-foreground"
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
  const [confirmOpen, setConfirmOpen] = React.useState(false);
  const isArchived = archiveActions.isArchived === true;
  const Icon = isArchived ? ArchiveRestore : Archive;
  const label = archiveActions.isPending
    ? isArchived
      ? "Unarchiving…"
      : "Archiving…"
    : isArchived
      ? "Unarchive agent"
      : "Archive agent";

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
  channelCount,
  isPending,
  managedAgent,
  onDelete,
  onExport,
}: {
  channelCount: number;
  isPending: boolean;
  managedAgent?: ManagedAgent;
  onDelete: () => void;
  onExport?: () => void;
}) {
  const [confirmOpen, setConfirmOpen] = React.useState(false);
  const memoryQuery = useAgentMemoryQuery(managedAgent?.pubkey, {
    enabled: confirmOpen && managedAgent !== undefined,
  });
  const memoryCount =
    memoryQuery.data &&
    (memoryQuery.data.core ? 1 : 0) + memoryQuery.data.memories.length;

  return (
    <>
      <ProfileAgentActionRow
        destructive
        disabled={isPending}
        icon={Trash2}
        label="Delete agent"
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
          channelCount={channelCount}
          isPending={isPending}
          memoryCount={memoryCount}
          memoriesLoading={memoryQuery.isLoading}
          onConfirm={() => {
            setConfirmOpen(false);
            onDelete();
          }}
          onExport={onExport}
          onOpenChange={setConfirmOpen}
          open={confirmOpen}
        />
      ) : null}
    </>
  );
}

function AgentDeleteConfirmDialog({
  agent,
  channelCount,
  isPending,
  memoryCount,
  memoriesLoading,
  onConfirm,
  onExport,
  onOpenChange,
  open,
}: {
  agent: ManagedAgent;
  channelCount: number;
  isPending: boolean;
  memoryCount?: number;
  memoriesLoading: boolean;
  onConfirm: () => void;
  onExport?: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const isProviderAgent = agent.backend.type === "provider";
  const showExportRecommendation =
    onExport !== undefined && memoryCount !== undefined && memoryCount > 0;

  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent data-testid="agent-delete-confirm-dialog">
        <AlertDialogHeader>
          <AlertDialogTitle>Delete {agent.name}?</AlertDialogTitle>
          <AlertDialogDescription>
            This permanently removes the agent, its saved key, and its channel
            access.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="grid grid-cols-2 gap-3">
          <div className="rounded-lg border bg-muted/30 px-4 py-3">
            <p className="text-xs text-muted-foreground">Memories</p>
            <p
              className="mt-1 text-2xl font-semibold tabular-nums"
              data-testid="agent-delete-memory-count"
            >
              {memoriesLoading ? "…" : (memoryCount ?? "—")}
            </p>
          </div>
          <div className="rounded-lg border bg-muted/30 px-4 py-3">
            <p className="text-xs text-muted-foreground">Channels</p>
            <p
              className="mt-1 text-2xl font-semibold tabular-nums"
              data-testid="agent-delete-channel-count"
            >
              {channelCount}
            </p>
          </div>
        </div>
        {showExportRecommendation ? (
          <p className="text-sm text-muted-foreground">
            Want a backup?{" "}
            <Button
              className="h-auto p-0 align-baseline font-medium underline underline-offset-2"
              data-testid="agent-delete-export-action"
              onClick={() => {
                onOpenChange(false);
                onExport();
              }}
              type="button"
              variant="link"
            >
              Export this agent
            </Button>
            .
          </p>
        ) : null}
        {isProviderAgent ? (
          <p className="text-sm text-muted-foreground">
            The remote process may keep running if Buzz can&apos;t reach it.
          </p>
        ) : null}
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
            {isPending ? "Deleting…" : "Delete agent"}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
