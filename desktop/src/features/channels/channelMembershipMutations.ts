import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  addChannelMembers,
  invokeTauri,
  joinChannel,
  leaveChannel,
  removeChannelMember,
} from "@/shared/api/tauri";
import type { AddChannelMembersInput } from "@/shared/api/types";
import { invalidateChannelState } from "./hooks";
import { refreshDirectoryAfterMembershipChange } from "./membershipDirectorySync";

// Accepted membership writes retire directory reads through the same bounded
// owner as live events. Settled channel invalidation still handles failures.
export function useAddChannelMembersMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (
      input: Omit<AddChannelMembersInput, "channelId"> & {
        channelId?: string;
      },
    ) => {
      const { channelId: capturedChannelId, ...rest } = input;
      const effectiveChannelId = capturedChannelId ?? channelId;
      if (!effectiveChannelId) {
        throw new Error("No channel selected.");
      }

      return addChannelMembers({ ...rest, channelId: effectiveChannelId });
    },
    onSuccess: (result, variables) => {
      if (result.added.length > 0) {
        refreshDirectoryAfterMembershipChange(queryClient);
      }
      const effectiveChannelId = variables.channelId ?? channelId;
      if (
        effectiveChannelId &&
        variables.role === "bot" &&
        result.added.length > 0
      ) {
        void invokeTauri("sync_agents_to_active_huddle", {
          channelId: effectiveChannelId,
          agentPubkeys: result.added,
        }).catch((error) => {
          console.warn("Could not sync added agents into Huddle:", error);
        });
      }
    },
    onSettled: async (_data, _err, variables) => {
      // Invalidate the effective channel (the one actually mutated) not the
      // live hook-closure channel, which may have changed mid-send.
      const effectiveChannelId = variables?.channelId ?? channelId;
      await invalidateChannelState(queryClient, effectiveChannelId);
    },
  });
}

export function useRemoveChannelMemberMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (pubkey: string) => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      await removeChannelMember(channelId, pubkey);
    },
    onSuccess: () => refreshDirectoryAfterMembershipChange(queryClient),
    onSettled: async () => {
      await Promise.all([
        invalidateChannelState(queryClient, channelId),
        queryClient.invalidateQueries({ queryKey: ["managed-agents"] }),
      ]);
    },
  });
}

export function useJoinChannelMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async () => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      await joinChannel(channelId);
    },
    onSuccess: () => refreshDirectoryAfterMembershipChange(queryClient),
    onSettled: async () => {
      await invalidateChannelState(queryClient, channelId);
    },
  });
}

export function useLeaveChannelMutation(channelId: string | null) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async () => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      await leaveChannel(channelId);
    },
    onSuccess: () => refreshDirectoryAfterMembershipChange(queryClient),
    onSettled: async () => {
      await invalidateChannelState(queryClient, channelId);
    },
  });
}
