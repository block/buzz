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
import { useTranslation } from "@/i18n";
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
  const { t } = useTranslation();
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
          label={t("profile.agent-management.duplicate")}
          onClick={onDuplicateAgent}
          testId="user-profile-duplicate-agent-row"
        />
      ) : null}
      {onExportAgent ? (
        <ProfileAgentActionRow
          disabled={isDeletePending}
          icon={Download}
          label={t("profile.agent-management.export")}
          onClick={onExportAgent}
          testId="user-profile-export-agent-row"
        />
      ) : null}
      {onCreateCard ? (
        <ProfileAgentActionRow
          disabled={isDeletePending}
          icon={Sparkles}
          label={t("profile.agent-management.create-card")}
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
  const { t } = useTranslation();
  const [confirmOpen, setConfirmOpen] = React.useState(false);
  const isArchived = archiveActions.isArchived === true;
  const Icon = isArchived ? ArchiveRestore : Archive;
  const label = archiveActions.isPending
    ? isArchived
      ? t("profile.agent-management.unarchiving")
      : t("profile.agent-management.archiving")
    : isArchived
      ? t("profile.agent-management.unarchive")
      : t("profile.agent-management.archive");

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
  const { t } = useTranslation();
  const [confirmOpen, setConfirmOpen] = React.useState(false);

  return (
    <>
      <ProfileAgentActionRow
        destructive
        disabled={isPending}
        icon={Trash2}
        label={t("profile.agent-management.delete-agent")}
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
  const { t } = useTranslation();
  const isProviderAgent = agent.backend.type === "provider";

  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      <AlertDialogContent data-testid="agent-delete-confirm-dialog">
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t("profile.agent-management.delete-title")}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {isProviderAgent
              ? t("profile.agent-management.delete-desc-provider")
              : t("profile.agent-management.delete-desc-local")}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <ul className="list-disc space-y-1.5 pl-5 text-sm text-muted-foreground">
          <li>{t("profile.agent-management.delete-list-record")}</li>
          <li>{t("profile.agent-management.delete-list-channels")}</li>
          <li>{t("profile.agent-management.delete-list-relay")}</li>
          <li>
            {isProviderAgent
              ? t("profile.agent-management.delete-list-provider")
              : t("profile.agent-management.delete-list-local")}
          </li>
        </ul>
        <p className="text-sm text-muted-foreground">
          {t("profile.agent-management.delete-archive-hint")}
        </p>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button type="button" variant="outline">
              {t("profile.agent-management.cancel")}
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction
            className={buttonVariants({ variant: "destructive" })}
            data-testid="agent-delete-confirm-action"
            disabled={isPending}
            onClick={onConfirm}
          >
            {isPending
              ? t("profile.agent-management.deleting")
              : t("profile.agent-management.delete-agent")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
