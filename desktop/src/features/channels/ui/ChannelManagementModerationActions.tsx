import { Archive, ArchiveRestore, Trash2 } from "lucide-react";

import { cn } from "@/shared/lib/cn";
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
        <AlertDialog
          onOpenChange={handleDeleteDialogOpenChange}
          open={isDeleteDialogOpen}
        >
          <AlertDialogTrigger asChild>
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
          </AlertDialogTrigger>
          <AlertDialogContent data-testid="channel-delete-confirmation-dialog">
            <AlertDialogHeader>
              <AlertDialogTitle>Delete channel?</AlertDialogTitle>
              <AlertDialogDescription>
                Delete {resolvedChannelName} from the workspace list. This
                action cannot be undone.
              </AlertDialogDescription>
            </AlertDialogHeader>
            {deleteChannelMutation.error instanceof Error ? (
              <p className="text-sm text-destructive">
                {deleteChannelMutation.error.message}
              </p>
            ) : null}
            <AlertDialogFooter>
              <AlertDialogCancel asChild>
                <Button
                  data-testid="channel-delete-cancel"
                  disabled={deleteChannelMutation.isPending}
                  type="button"
                  variant="outline"
                >
                  Cancel
                </Button>
              </AlertDialogCancel>
              <AlertDialogAction asChild>
                <Button
                  data-testid="channel-delete-confirm"
                  disabled={deleteChannelMutation.isPending}
                  onClick={(event) => {
                    event.preventDefault();
                    void handleDeleteChannel();
                  }}
                  type="button"
                  variant="destructive"
                >
                  {deleteChannelMutation.isPending
                    ? "Deleting..."
                    : "Delete channel"}
                </Button>
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </div>
  );
}
