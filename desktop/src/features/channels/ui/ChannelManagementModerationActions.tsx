import { Archive, ArchiveRestore, Trash2 } from "lucide-react";
import { useMemo, type ReactNode } from "react";

import { ownsAuthorAgent } from "@/features/profile/lib/identity";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { ChannelMember } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

type ChannelMutation<TArgs = void> = {
  error: unknown;
  isPending: boolean;
  mutateAsync: (args: TArgs) => Promise<unknown>;
};

type ChannelManagementModerationActionsProps = {
  archiveChannelMutation: ChannelMutation;
  canManageChannel: boolean;
  deleteChannelMutation: ChannelMutation;
  handleDeleteChannel: () => Promise<void>;
  handleDeleteDialogOpenChange: (open: boolean) => void;
  isArchived: boolean;
  isDark: boolean;
  isDeleteDialogOpen: boolean;
  canDeleteChannel: boolean;
  resolvedChannelName: string;
  unarchiveChannelMutation: ChannelMutation;
};

type ChannelDeleteConfirmationDialogProps = {
  channelName: string;
  error: unknown;
  isPending: boolean;
  onConfirm: () => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  trigger?: ReactNode;
};

export function useChannelModerationCapabilities(
  members: ChannelMember[] | undefined,
  currentPubkey: string | undefined,
  enabled: boolean,
) {
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : undefined;
  const selfRole = members?.find(
    (member) => normalizePubkey(member.pubkey) === normalizedCurrentPubkey,
  )?.role;
  const ownerMemberPubkeys = useMemo(
    () =>
      (members ?? [])
        .filter(
          (member) =>
            member.role === "owner" &&
            normalizePubkey(member.pubkey) !== normalizedCurrentPubkey,
        )
        .map((member) => member.pubkey),
    [members, normalizedCurrentPubkey],
  );
  const shouldResolveAgentOwnership =
    enabled && selfRole !== "owner" && ownerMemberPubkeys.length > 0;
  const ownerProfilesQuery = useUsersBatchQuery(ownerMemberPubkeys, {
    enabled: shouldResolveAgentOwnership,
  });
  const canManageOwnedAgentChannel = ownerMemberPubkeys.some((pubkey) =>
    ownsAuthorAgent(
      ownerProfilesQuery.data?.profiles[normalizePubkey(pubkey)],
      currentPubkey,
    ),
  );

  return {
    canDeleteChannel: selfRole === "owner" || canManageOwnedAgentChannel,
    canManageChannel:
      selfRole === "owner" ||
      selfRole === "admin" ||
      canManageOwnedAgentChannel,
    error: shouldResolveAgentOwnership ? ownerProfilesQuery.error : null,
    isLoading: shouldResolveAgentOwnership && ownerProfilesQuery.isLoading,
  };
}

export function ChannelDeleteConfirmationDialog({
  channelName,
  error,
  isPending,
  onConfirm,
  onOpenChange,
  open,
  trigger,
}: ChannelDeleteConfirmationDialogProps) {
  return (
    <AlertDialog onOpenChange={onOpenChange} open={open}>
      {trigger ? (
        <AlertDialogTrigger asChild>{trigger}</AlertDialogTrigger>
      ) : null}
      <AlertDialogContent data-testid="channel-delete-confirmation-dialog">
        <AlertDialogHeader>
          <AlertDialogTitle>Delete channel?</AlertDialogTitle>
          <AlertDialogDescription>
            Delete {channelName} from the community list. This action cannot be
            undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {error instanceof Error ? (
          <p className="text-sm text-destructive">{error.message}</p>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button
              data-testid="channel-delete-cancel"
              disabled={isPending}
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction asChild>
            <Button
              data-testid="channel-delete-confirm"
              disabled={isPending}
              onClick={(event) => {
                event.preventDefault();
                onConfirm();
              }}
              type="button"
              variant="destructive"
            >
              {isPending ? "Deleting..." : "Delete channel"}
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

export function ChannelManagementModerationActions({
  archiveChannelMutation,
  canManageChannel,
  deleteChannelMutation,
  handleDeleteChannel,
  handleDeleteDialogOpenChange,
  isArchived,
  isDark,
  isDeleteDialogOpen,
  canDeleteChannel,
  resolvedChannelName,
  unarchiveChannelMutation,
}: ChannelManagementModerationActionsProps) {
  return (
    <div
      className={cn(
        "absolute bottom-3 right-3 z-20 flex items-center gap-2 rounded-full border border-border/60 p-1 shadow-sm",
        isDark
          ? "bg-background/80 backdrop-blur-xl supports-[backdrop-filter]:bg-background/70"
          : "bg-background/90 backdrop-blur-md supports-[backdrop-filter]:bg-background/80",
      )}
      data-testid="channel-management-footer"
    >
      {isArchived ? (
        <Button
          aria-label={
            unarchiveChannelMutation.isPending
              ? "Restoring channel"
              : "Unarchive channel"
          }
          data-testid="channel-management-unarchive"
          disabled={!canManageChannel || unarchiveChannelMutation.isPending}
          onClick={() => {
            void unarchiveChannelMutation.mutateAsync();
          }}
          size="icon"
          title={
            unarchiveChannelMutation.isPending
              ? "Restoring channel"
              : "Unarchive channel"
          }
          type="button"
          variant="ghost"
        >
          <ArchiveRestore className="h-4 w-4" />
        </Button>
      ) : (
        <Button
          aria-label={
            archiveChannelMutation.isPending
              ? "Archiving channel"
              : "Archive channel"
          }
          data-testid="channel-management-archive"
          disabled={!canManageChannel || archiveChannelMutation.isPending}
          onClick={() => {
            void archiveChannelMutation.mutateAsync();
          }}
          size="icon"
          title={
            archiveChannelMutation.isPending
              ? "Archiving channel"
              : "Archive channel"
          }
          type="button"
          variant="ghost"
        >
          <Archive className="h-4 w-4" />
        </Button>
      )}
      {canDeleteChannel ? (
        <ChannelDeleteConfirmationDialog
          channelName={resolvedChannelName}
          error={deleteChannelMutation.error}
          isPending={deleteChannelMutation.isPending}
          onConfirm={() => {
            void handleDeleteChannel();
          }}
          onOpenChange={handleDeleteDialogOpenChange}
          open={isDeleteDialogOpen}
          trigger={
            <Button
              aria-label="Delete channel"
              data-testid="channel-management-delete"
              disabled={deleteChannelMutation.isPending}
              size="icon"
              title="Delete channel"
              type="button"
              variant="ghost"
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          }
        />
      ) : null}
    </div>
  );
}
