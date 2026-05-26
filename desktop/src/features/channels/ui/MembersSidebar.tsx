import * as React from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  useAddChannelMembersMutation,
  useChannelMembersQuery,
} from "@/features/channels/hooks";
import { useUpdateManagedAgentMutation } from "@/features/agents/hooks";
import { CreateAgentRespondToField } from "@/features/agents/ui/RespondToField";
import { useClassifiedMembers } from "@/features/channels/lib/useClassifiedMembers";
import {
  formatMemberName,
  formatPubkey,
} from "@/features/channels/lib/memberUtils";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { usePresenceQuery } from "@/features/presence/hooks";
import { changeChannelMemberRole } from "@/shared/api/tauri";
import type {
  Channel,
  ChannelMember,
  ManagedAgent,
  RespondToMode,
} from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { useFeedbackToasts } from "@/shared/hooks/useToastEffect";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/shared/ui/sheet";
import { MembersSidebarAgentControls } from "./MembersSidebarAgentControls";
import { ChannelMemberInviteCard } from "./ChannelMemberInviteCard";
import { MembersSidebarMemberCard } from "./MembersSidebarMemberCard";
import { useMembersSidebarActions } from "./useMembersSidebarActions";

type MembersSidebarProps = {
  channel: Channel | null;
  currentPubkey?: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onViewActivity?: (pubkey: string) => void;
};

export function MembersSidebar({
  channel,
  currentPubkey,
  open,
  onOpenChange,
  onViewActivity,
}: MembersSidebarProps) {
  const channelId = channel?.id ?? null;
  const queryClient = useQueryClient();
  const membersQuery = useChannelMembersQuery(channelId, open);
  const addMembersMutation = useAddChannelMembersMutation(channelId);
  const changeRoleMutation = useMutation({
    mutationFn: async ({ pubkey, role }: { pubkey: string; role: string }) => {
      if (!channelId) throw new Error("No channel selected.");
      await changeChannelMemberRole(channelId, pubkey, role);
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({
        queryKey: ["channels", channelId],
      });
    },
  });
  const changeRoleError =
    changeRoleMutation.error instanceof Error
      ? changeRoleMutation.error.message
      : null;

  const rawMembers = membersQuery.data ?? [];
  const { people, bots, archived, isBot, isMyBot, managedAgentsQuery } =
    useClassifiedMembers(rawMembers, currentPubkey);

  const allMemberPubkeys = React.useMemo(
    () => rawMembers.map((member) => member.pubkey),
    [rawMembers],
  );
  const memberPresenceQuery = usePresenceQuery(allMemberPubkeys, {
    enabled: open && rawMembers.length > 0,
  });
  const memberProfilesQuery = useUsersBatchQuery(allMemberPubkeys, {
    enabled: open && rawMembers.length > 0,
  });

  const selfMember =
    rawMembers.find((member) => member.pubkey === currentPubkey) ?? null;
  const canManageMembers =
    selfMember?.role === "owner" || selfMember?.role === "admin";
  const isArchived =
    channel?.archivedAt !== null && channel?.archivedAt !== undefined;
  const managedAgentByPubkey = React.useMemo(
    () =>
      new Map(
        (managedAgentsQuery.data ?? []).map((agent) => [
          normalizePubkey(agent.pubkey),
          agent,
        ]),
      ),
    [managedAgentsQuery.data],
  );
  const controllableManagedBots = React.useMemo(
    () =>
      bots.flatMap((member) => {
        const agent = managedAgentByPubkey.get(normalizePubkey(member.pubkey));
        return agent ? [agent] : [];
      }),
    [bots, managedAgentByPubkey],
  );
  const canRemoveMember = React.useCallback(
    (member: ChannelMember) => {
      return (
        (selfMember?.role === "admin" && member.pubkey !== currentPubkey) ||
        (selfMember?.role === "owner" && member.role !== "owner") ||
        Boolean(selfMember && isMyBot(member)) ||
        member.pubkey === currentPubkey
      );
    },
    [currentPubkey, isMyBot, selfMember],
  );
  const removableManagedBots = React.useMemo(
    () =>
      bots.flatMap((member) => {
        if (!canRemoveMember(member)) {
          return [];
        }

        const agent = managedAgentByPubkey.get(normalizePubkey(member.pubkey));
        return agent ? [agent] : [];
      }),
    [bots, canRemoveMember, managedAgentByPubkey],
  );
  const {
    actionErrorMessage,
    actionNoticeMessage,
    handleLifecycleAction: handleAgentLifecycleAction,
    handleRemoveAll,
    handleRemoveMember,
    handleRespawnAll,
    handleStopAll,
    hasControllableManagedBots,
    hasRemovableManagedBots,
    hasStoppableManagedBots,
    isActionPending,
  } = useMembersSidebarActions({
    channelId,
    controllableManagedBots,
    removableManagedBots,
    currentPubkey,
    onOpenChange,
  });

  useFeedbackToasts(actionNoticeMessage, actionErrorMessage);

  const [editRespondToAgent, setEditRespondToAgent] =
    React.useState<ManagedAgent | null>(null);

  if (!channel) {
    return null;
  }

  function renderMemberCard(member: ChannelMember, memberIsBot: boolean) {
    return (
      <MembersSidebarMemberCard
        canChangeRole={canManageMembers && member.pubkey !== currentPubkey}
        canRemoveMember={canRemoveMember(member)}
        isActionPending={isActionPending || changeRoleMutation.isPending}
        isArchived={isArchived}
        key={member.pubkey}
        managedAgent={
          memberIsBot
            ? managedAgentByPubkey.get(normalizePubkey(member.pubkey))
            : undefined
        }
        member={member}
        memberIsBot={memberIsBot}
        memberAvatarLabel={member.displayName ?? formatPubkey(member.pubkey)}
        memberLabel={formatMemberName(member, currentPubkey)}
        onChangeRole={(m, role) => {
          void changeRoleMutation.mutateAsync({ pubkey: m.pubkey, role });
        }}
        onEditRespondTo={memberIsBot ? setEditRespondToAgent : undefined}
        onManagedAgentAction={(agent) => {
          void handleAgentLifecycleAction(agent);
        }}
        onRemoveMember={handleRemoveMember}
        onViewActivity={
          onViewActivity
            ? (pubkey: string) => {
                onOpenChange(false);
                onViewActivity(pubkey);
              }
            : undefined
        }
        presenceStatus={
          memberPresenceQuery.data?.[member.pubkey.toLowerCase()] ?? null
        }
        profileAvatarUrl={
          memberProfilesQuery.data?.profiles[member.pubkey.toLowerCase()]
            ?.avatarUrl ?? null
        }
      />
    );
  }

  return (
    <>
      <Sheet onOpenChange={onOpenChange} open={open}>
        <SheetContent
          className="flex w-full flex-col gap-0 overflow-hidden border-l border-border/80 bg-background p-0 sm:max-w-md"
          data-testid="members-sidebar"
          side="right"
        >
          <SheetHeader className="relative z-10 space-y-2 bg-background/25 px-6 py-6 text-left shadow-[0_4px_24px_rgba(0,0,0,0.06)] backdrop-blur-xl supports-[backdrop-filter]:bg-background/20 dark:shadow-[0_4px_24px_rgba(0,0,0,0.25)]">
            <SheetTitle>Members</SheetTitle>
            <SheetDescription>
              People and bots in {channel.name}.
            </SheetDescription>
          </SheetHeader>

          <div className="flex-1 space-y-6 overflow-y-auto px-6 py-6">
            {(canManageMembers || channel.visibility === "open") &&
            channel.channelType !== "dm" ? (
              <ChannelMemberInviteCard
                existingMembers={rawMembers}
                isPending={addMembersMutation.isPending}
                onSubmit={(input) => addMembersMutation.mutateAsync(input)}
                open={open}
                requestErrorMessage={
                  addMembersMutation.error instanceof Error
                    ? addMembersMutation.error.message
                    : null
                }
              />
            ) : null}

            <section className="space-y-2.5">
              <div className="flex items-center justify-between gap-2">
                <h2 className="text-sm font-semibold tracking-tight">People</h2>
                <span className="rounded-full bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground">
                  {people.length}
                </span>
              </div>
              <div className="space-y-2" data-testid="members-sidebar-people">
                {people.length > 0 ? (
                  people.map((member) => renderMemberCard(member, false))
                ) : (
                  <p className="text-sm text-muted-foreground">
                    {membersQuery.isLoading
                      ? "Loading members..."
                      : "No people found."}
                  </p>
                )}
              </div>
            </section>

            <section className="space-y-2.5">
              <div className="flex items-center gap-2">
                <h2 className="text-sm font-semibold tracking-tight">Bots</h2>
                <span className="rounded-full bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground">
                  {bots.length}
                </span>
                {hasControllableManagedBots ? (
                  <MembersSidebarAgentControls
                    canBulkRemove={hasRemovableManagedBots}
                    canBulkRespawn={hasControllableManagedBots}
                    canBulkStop={hasStoppableManagedBots}
                    disabled={isActionPending || isArchived}
                    onRemoveAll={() => {
                      void handleRemoveAll();
                    }}
                    onRespawnAll={() => {
                      void handleRespawnAll();
                    }}
                    onStopAll={() => {
                      void handleStopAll();
                    }}
                  />
                ) : null}
              </div>
              <div className="space-y-2" data-testid="members-sidebar-bots">
                {bots.length > 0 ? (
                  bots.map((member) => renderMemberCard(member, true))
                ) : (
                  <p className="text-sm text-muted-foreground">
                    {membersQuery.isLoading
                      ? "Loading members..."
                      : "No bots found."}
                  </p>
                )}
              </div>
            </section>

            {archived.length > 0 ? (
              <section className="space-y-2.5">
                <details
                  className="group/archived"
                  data-testid="members-sidebar-archived"
                >
                  <summary className="flex cursor-pointer items-center gap-2 list-none [&::-webkit-details-marker]:hidden">
                    <h2 className="text-sm font-semibold tracking-tight text-muted-foreground">
                      Archived
                    </h2>
                    <span
                      className="rounded-full bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground"
                      data-testid="members-sidebar-archived-count"
                    >
                      {archived.length}
                    </span>
                    <span className="ml-auto text-xs text-muted-foreground transition-transform group-open/archived:rotate-90">
                      ▸
                    </span>
                  </summary>
                  <div
                    className="mt-2 space-y-2"
                    data-testid="members-sidebar-archived-list"
                  >
                    {archived.map((member) =>
                      renderMemberCard(member, isBot(member)),
                    )}
                  </div>
                </details>
              </section>
            ) : null}

            {changeRoleError ? (
              <p
                className="text-sm text-destructive"
                data-testid="members-sidebar-action-error"
              >
                {changeRoleError}
              </p>
            ) : null}
          </div>
        </SheetContent>
      </Sheet>
      <EditRespondToDialog
        agent={editRespondToAgent}
        currentPubkey={currentPubkey}
        onOpenChange={(dialogOpen) => {
          if (!dialogOpen) setEditRespondToAgent(null);
        }}
        open={editRespondToAgent !== null}
      />
    </>
  );
}

function EditRespondToDialog({
  agent,
  currentPubkey,
  onOpenChange,
  open,
}: {
  agent: ManagedAgent | null;
  currentPubkey?: string;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const updateMutation = useUpdateManagedAgentMutation();
  const [respondTo, setRespondTo] = React.useState<RespondToMode>("owner-only");
  const [respondToAllowlist, setRespondToAllowlist] = React.useState<string[]>(
    [],
  );

  React.useEffect(() => {
    if (agent) {
      setRespondTo(agent.respondTo);
      setRespondToAllowlist([...agent.respondToAllowlist]);
    }
  }, [agent]);

  const respondToValid =
    respondTo !== "allowlist" || respondToAllowlist.length > 0;

  async function handleSave() {
    if (!agent) return;
    await updateMutation.mutateAsync({
      pubkey: agent.pubkey,
      respondTo,
      respondToAllowlist:
        respondTo === "allowlist" ? respondToAllowlist : undefined,
    });
    onOpenChange(false);
  }

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Edit respond-to</DialogTitle>
          <DialogDescription>
            Choose who {agent?.name ?? "this agent"} responds to.
          </DialogDescription>
        </DialogHeader>
        <CreateAgentRespondToField
          allowlist={respondToAllowlist}
          disabled={updateMutation.isPending}
          mode={respondTo}
          onAllowlistChange={setRespondToAllowlist}
          onModeChange={setRespondTo}
          ownerPubkey={currentPubkey}
        />
        {updateMutation.error instanceof Error ? (
          <p className="text-sm text-destructive">
            {updateMutation.error.message}
          </p>
        ) : null}
        <div className="flex justify-end gap-2">
          <Button
            onClick={() => onOpenChange(false)}
            size="sm"
            type="button"
            variant="outline"
          >
            Cancel
          </Button>
          <Button
            disabled={!respondToValid || updateMutation.isPending}
            onClick={() => void handleSave()}
            size="sm"
            type="button"
          >
            {updateMutation.isPending ? "Saving..." : "Save"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
