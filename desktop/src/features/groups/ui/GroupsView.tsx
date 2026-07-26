import { MoreHorizontal, Plus, Users } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useChannelsQuery } from "@/features/channels/hooks";
import { useMyRelayMembershipQuery } from "@/features/community-members/hooks";
import {
  useCreateGroupMutation,
  useDeleteGroupMutation,
  useGroupsQuery,
  useUpdateGroupMutation,
} from "@/features/groups/groupHooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { UserGroup } from "@/shared/api/relayGroups";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { UserAvatar } from "@/shared/ui/UserAvatar";
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
import { GroupDialog } from "./GroupDialog";

type DialogState =
  | { mode: "create"; group: null }
  | { mode: "edit"; group: UserGroup }
  | null;

export function GroupsView() {
  const groupsQuery = useGroupsQuery();
  const channelsQuery = useChannelsQuery();
  const identityQuery = useIdentityQuery();
  const membershipQuery = useMyRelayMembershipQuery();
  const createMutation = useCreateGroupMutation();
  const updateMutation = useUpdateGroupMutation();
  const deleteMutation = useDeleteGroupMutation();
  const [dialogState, setDialogState] = React.useState<DialogState>(null);
  const [membersGroup, setMembersGroup] = React.useState<UserGroup | null>(
    null,
  );
  const [deleteGroup, setDeleteGroup] = React.useState<UserGroup | null>(null);
  const groups = groupsQuery.data ?? [];
  const allMemberPubkeys = React.useMemo(
    () => [...new Set(groups.flatMap((group) => group.memberPubkeys))],
    [groups],
  );
  const profilesQuery = useUsersBatchQuery(allMemberPubkeys, {
    enabled: allMemberPubkeys.length > 0,
  });
  const viewerPubkey = identityQuery.data?.pubkey.toLowerCase() ?? "";
  const relayRole = membershipQuery.data?.role;
  const isCommunityAdmin = relayRole === "owner" || relayRole === "admin";
  const mutationError =
    createMutation.error instanceof Error
      ? createMutation.error
      : updateMutation.error instanceof Error
        ? updateMutation.error
        : null;
  const isSaving = createMutation.isPending || updateMutation.isPending;

  function canManage(group: UserGroup): boolean {
    return isCommunityAdmin || group.creator.toLowerCase() === viewerPubkey;
  }

  async function submitGroup(next: UserGroup) {
    if (dialogState?.mode === "edit") {
      await updateMutation.mutateAsync({
        group: dialogState.group,
        next,
      });
    } else {
      await createMutation.mutateAsync(next);
    }
    setDialogState(null);
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <header className="flex items-center justify-between gap-4 border-b border-border/60 px-6 py-5">
        <div>
          <h1 className="text-xl font-semibold">Groups</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Shared handles for mentioning people and agents together.
          </p>
        </div>
        <Button
          data-testid="new-group-button"
          onClick={() => setDialogState({ mode: "create", group: null })}
          size="sm"
        >
          <Plus className="h-4 w-4" />
          New group
        </Button>
      </header>

      <main className="flex-1 px-6 py-6">
        {groupsQuery.isLoading ? (
          <p className="text-sm text-muted-foreground">Loading groups…</p>
        ) : groupsQuery.error instanceof Error ? (
          <p className="text-sm text-destructive">
            {groupsQuery.error.message}
          </p>
        ) : groups.length === 0 ? (
          <div className="flex min-h-72 flex-col items-center justify-center rounded-2xl border border-dashed border-border/80 px-6 text-center">
            <span className="mb-4 flex h-12 w-12 items-center justify-center rounded-full bg-primary/10 text-primary">
              <Users className="h-6 w-6" />
            </span>
            <h2 className="text-base font-semibold">
              Mention everyone at once
            </h2>
            <p className="mt-2 max-w-md text-sm text-muted-foreground">
              Groups let you mention many people and agents at once with one
              memorable handle.
            </p>
            <Button
              className="mt-5"
              onClick={() => setDialogState({ mode: "create", group: null })}
              size="sm"
            >
              <Plus className="h-4 w-4" />
              Create a group
            </Button>
          </div>
        ) : (
          <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
            {groups.map((group) => (
              <Card
                className="flex min-h-48 flex-col p-5"
                data-testid={`group-card-${group.id}`}
                key={group.id}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <h2 className="truncate text-base font-semibold">
                      {group.name}
                    </h2>
                    <p className="mt-0.5 truncate text-sm text-primary">
                      @{group.handle}
                    </p>
                  </div>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button
                        aria-label={`Actions for ${group.name}`}
                        size="icon"
                        variant="ghost"
                      >
                        <MoreHorizontal className="h-4 w-4" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem onSelect={() => setMembersGroup(group)}>
                        View members
                      </DropdownMenuItem>
                      {canManage(group) ? (
                        <>
                          <DropdownMenuSeparator />
                          <DropdownMenuItem
                            onSelect={() =>
                              setDialogState({ mode: "edit", group })
                            }
                          >
                            Edit
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            className="text-destructive focus:text-destructive"
                            onSelect={() => setDeleteGroup(group)}
                          >
                            Delete
                          </DropdownMenuItem>
                        </>
                      ) : null}
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>

                <p className="mt-3 line-clamp-2 min-h-10 text-sm text-muted-foreground">
                  {group.description || "No description"}
                </p>

                <div className="mt-auto flex items-end justify-between gap-3 pt-5">
                  <div className="flex items-center">
                    {group.memberPubkeys
                      .filter((_pubkey, index) => index < 4)
                      .map((pubkey, index) => {
                        const profile =
                          profilesQuery.data?.profiles[pubkey.toLowerCase()];
                        return (
                          <UserAvatar
                            avatarUrl={profile?.avatarUrl ?? null}
                            className={
                              index > 0 ? "-ml-2 ring-2 ring-card" : ""
                            }
                            displayName={profile?.displayName ?? pubkey}
                            key={pubkey}
                            size="sm"
                          />
                        );
                      })}
                    <span className="ml-2 text-xs text-muted-foreground">
                      {group.memberPubkeys.length} member
                      {group.memberPubkeys.length === 1 ? "" : "s"}
                    </span>
                  </div>
                  <span className="text-xs text-muted-foreground">
                    {group.defaultChannelIds.length} default channel
                    {group.defaultChannelIds.length === 1 ? "" : "s"}
                  </span>
                </div>
              </Card>
            ))}
          </div>
        )}
      </main>

      <GroupDialog
        channels={channelsQuery.data ?? []}
        error={mutationError}
        group={dialogState?.group ?? null}
        isPending={isSaving}
        mode={dialogState?.mode ?? "create"}
        onOpenChange={(open) => {
          if (!open) {
            createMutation.reset();
            updateMutation.reset();
            setDialogState(null);
          }
        }}
        onSubmit={submitGroup}
        open={dialogState !== null}
        viewerPubkey={viewerPubkey}
      />

      <Dialog
        onOpenChange={(open) => !open && setMembersGroup(null)}
        open={membersGroup !== null}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{membersGroup?.name} members</DialogTitle>
            <DialogDescription>
              Everyone notified when @{membersGroup?.handle} is mentioned in a
              channel they belong to.
            </DialogDescription>
          </DialogHeader>
          <div className="max-h-80 space-y-2 overflow-y-auto">
            {membersGroup?.memberPubkeys.length ? (
              membersGroup.memberPubkeys.map((pubkey) => {
                const profile =
                  profilesQuery.data?.profiles[pubkey.toLowerCase()];
                return (
                  <div
                    className="flex items-center gap-3 rounded-lg border border-border/70 px-3 py-2"
                    key={pubkey}
                  >
                    <UserAvatar
                      avatarUrl={profile?.avatarUrl ?? null}
                      displayName={profile?.displayName ?? pubkey}
                      size="sm"
                    />
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium">
                        {profile?.displayName ?? pubkey}
                      </p>
                      {profile?.isAgent ? (
                        <p className="text-xs text-muted-foreground">agent</p>
                      ) : null}
                    </div>
                  </div>
                );
              })
            ) : (
              <p className="py-4 text-center text-sm text-muted-foreground">
                This group has no members.
              </p>
            )}
          </div>
        </DialogContent>
      </Dialog>

      <AlertDialog
        onOpenChange={(open) => !open && setDeleteGroup(null)}
        open={deleteGroup !== null}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete {deleteGroup?.name}?</AlertDialogTitle>
            <AlertDialogDescription>
              The @{deleteGroup?.handle} handle will stop working. Existing
              messages keep their rendered group mention.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              disabled={deleteMutation.isPending}
              onClick={() => {
                if (!deleteGroup) return;
                void deleteMutation
                  .mutateAsync(deleteGroup.id)
                  .then(() => {
                    setDeleteGroup(null);
                  })
                  .catch((error) => {
                    toast.error(
                      error instanceof Error
                        ? error.message
                        : "Could not delete the group.",
                    );
                  });
              }}
            >
              {deleteMutation.isPending ? "Deleting…" : "Delete"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
