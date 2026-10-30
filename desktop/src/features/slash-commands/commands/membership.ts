import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import {
  managedAgentsQueryKey,
  useAttachManagedAgentToChannelMutation,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import {
  channelsQueryKey,
  invalidateChannelState,
  sortChannels,
  useAddChannelMembersMutation,
  useChannelMembersQuery,
} from "@/features/channels/hooks";
import { openChannelDirectoryQueryKey } from "@/features/channels/openChannelDirectory";
import { channelMembersQueryKey } from "@/features/channels/rosterFreshness";
import { useCanAddChannelMembers } from "@/features/channels/useCanAddChannelMembers";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  getChannelMembers,
  getOpenChannelDirectory,
  joinChannel,
  leaveChannel,
  listManagedAgents,
  removeChannelMember,
} from "@/shared/api/tauri";
import { getUsersBatch } from "@/shared/api/tauriProfiles";
import type { Channel, ManagedAgent } from "@/shared/api/types";
import {
  findChannelByName,
  groupInvitees,
  parseChannelRefNames,
  planInvite,
  planRemove,
} from "../lib/membershipPlan";
import {
  CONFIRM_LEAVE_CHANNEL_UI,
  type ConfirmLeaveChannelRequest,
  OPEN_MEMBERS_UI,
} from "../lib/membershipUi";
import { SlashCommandCancelled } from "../lib/slashCommandCancelled";
import { SlashUsageError } from "../lib/slashUsageError";
import type {
  SlashCommandDefinition,
  SlashCommandEnvironment,
  SlashCommandRunContext,
} from "../lib/types";

const MANAGED_AGENTS_STALE_TIME_MS = 15_000;

function errorMessages(errors: readonly { error: string }[]) {
  return errors.map((entry) => entry.error).join("; ");
}

export function useMembershipCommands(
  environment: SlashCommandEnvironment,
): readonly SlashCommandDefinition[] {
  const queryClient = useQueryClient();
  const channelId = environment.enabled ? environment.channelId : null;
  const canAddMembers = useCanAddChannelMembers(channelId);
  const membersQuery = useChannelMembersQuery(channelId);
  const selfPubkey = useIdentityQuery().data?.pubkey ?? null;
  useManagedAgentsQuery({ enabled: environment.enabled });
  const addMembers = useAddChannelMembersMutation(channelId).mutateAsync;
  const attachAgent =
    useAttachManagedAgentToChannelMutation(channelId).mutateAsync;

  const members = membersQuery.data;
  const selfRole = React.useMemo(() => {
    if (!selfPubkey || !members) return undefined;
    const self = selfPubkey.toLowerCase();
    return (
      members.find((member) => member.pubkey.toLowerCase() === self)?.role ??
      null
    );
  }, [members, selfPubkey]);
  const canManageMembers =
    selfRole === undefined || selfRole === "owner" || selfRole === "admin";

  return React.useMemo(() => {
    const freshRoster = async (id: string) =>
      (
        await queryClient.fetchQuery({
          queryKey: channelMembersQueryKey(id),
          queryFn: () => getChannelMembers(id, { readYourWrites: true }),
          staleTime: 0,
        })
      ).map((member) => member.pubkey);

    const requireMentions = (context: SlashCommandRunContext) => {
      if (context.mentionPubkeys.length === 0) throw new SlashUsageError();
      return context.mentionPubkeys;
    };

    const invite = async (context: SlashCommandRunContext) => {
      const mentioned = requireMentions(context);
      const invitees = planInvite(
        mentioned,
        await freshRoster(context.channelId),
      );
      const managed = await queryClient.fetchQuery({
        queryKey: managedAgentsQueryKey,
        queryFn: listManagedAgents,
        staleTime: MANAGED_AGENTS_STALE_TIME_MS,
      });
      const localManaged = new Map<string, ManagedAgent>(
        managed
          .filter((agent) => agent.backend.type === "local")
          .map((agent) => [agent.pubkey.toLowerCase(), agent]),
      );
      const unmanaged = invitees.filter((key) => !localManaged.has(key));
      const profiles: Record<string, { isAgent?: boolean }> =
        unmanaged.length > 0 ? (await getUsersBatch(unmanaged)).profiles : {};
      const groups = groupInvitees(
        invitees,
        localManaged,
        (key) => profiles[key]?.isAgent === true,
      );

      for (const agent of groups.attach) {
        await attachAgent({
          agent,
          channelId: context.channelId,
          ensureRunning: true,
        });
      }
      for (const [role, pubkeys] of [
        ["member", groups.humans],
        ["bot", groups.bots],
      ] as const) {
        if (pubkeys.length === 0) continue;
        const result = await addMembers({
          channelId: context.channelId,
          pubkeys,
          role,
        });
        if (result.errors.length > 0) {
          throw new Error(errorMessages(result.errors));
        }
      }
      context.toast.success(`Added ${invitees.length}`);
    };

    const remove = async (context: SlashCommandRunContext) => {
      const mentioned = requireMentions(context);
      const targets = planRemove(
        mentioned,
        await freshRoster(context.channelId),
        selfPubkey,
      );
      try {
        for (const pubkey of targets) {
          await removeChannelMember(context.channelId, pubkey);
        }
      } finally {
        await Promise.all([
          invalidateChannelState(queryClient, context.channelId),
          queryClient.invalidateQueries({ queryKey: ["managed-agents"] }),
          queryClient.invalidateQueries({ queryKey: ["relay-agents"] }),
        ]);
      }
      context.toast.success(`Removed ${targets.length}`);
    };

    const resolveJoinTarget = async (name: string) => {
      const known = [
        ...(queryClient.getQueryData<Channel[]>(channelsQueryKey) ?? []),
        ...(queryClient.getQueryData<Channel[]>(openChannelDirectoryQueryKey) ??
          []),
      ];
      const cached = findChannelByName(name, known);
      if (cached) return cached;
      const directory = await queryClient.fetchQuery({
        queryKey: openChannelDirectoryQueryKey,
        queryFn: async () => sortChannels(await getOpenChannelDirectory()),
        staleTime: 0,
      });
      return findChannelByName(name, directory);
    };

    const join = async (context: SlashCommandRunContext) => {
      const [name] = parseChannelRefNames(context.args);
      if (!name) throw new SlashUsageError();
      const target = await resolveJoinTarget(name);
      if (!target) throw new SlashUsageError(`No channel named #${name}`);
      if (!target.isMember) {
        if (target.archivedAt !== null) {
          throw new Error(`#${target.name} is archived`);
        }
        if (target.visibility !== "open") {
          throw new Error(`#${target.name} is private`);
        }
        await joinChannel(target.id);
        await invalidateChannelState(queryClient, target.id);
      }
      await context.navigation.goChannel(target.id);
    };

    const leave = async (context: SlashCommandRunContext) => {
      let settle: (confirmed: boolean) => void = () => {};
      const decision = new Promise<boolean>((resolve) => {
        settle = resolve;
      });
      const request: ConfirmLeaveChannelRequest = {
        channel: { id: context.channelId, name: context.channelName },
        settle,
      };
      if (!context.requestUi(CONFIRM_LEAVE_CHANNEL_UI, request)) {
        throw new Error("Could not open the leave confirmation.");
      }
      if (!(await decision)) throw new SlashCommandCancelled();
      await leaveChannel(context.channelId);
      await invalidateChannelState(queryClient, context.channelId);
      await context.navigation.goHome();
    };

    const who = (context: SlashCommandRunContext) => {
      if (!context.requestUi(OPEN_MEMBERS_UI)) {
        throw new Error("Could not open the member list.");
      }
    };

    return [
      {
        name: "invite",
        argumentHint: "@user…",
        description: "Add people to this channel",
        usage: "/invite @user…",
        context: "channel",
        isVisible: () => canAddMembers,
        run: invite,
      },
      {
        name: "remove",
        aliases: ["kick"],
        argumentHint: "@user…",
        description: "Remove people from this channel",
        usage: "/remove @user…",
        context: "channel",
        isVisible: () => canManageMembers,
        run: remove,
      },
      {
        name: "join",
        argumentHint: "#channel",
        description: "Join a channel and open it",
        usage: "/join #channel",
        context: "any",
        run: join,
      },
      {
        name: "leave",
        aliases: ["part"],
        description: "Leave this channel",
        usage: "/leave",
        context: "channel",
        run: leave,
      },
      {
        name: "who",
        aliases: ["members"],
        description: "Show who is in this channel",
        usage: "/who",
        context: "any",
        run: who,
      },
    ];
  }, [
    addMembers,
    attachAgent,
    canAddMembers,
    canManageMembers,
    queryClient,
    selfPubkey,
  ]);
}
