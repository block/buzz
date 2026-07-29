import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { attachManagedAgentToChannel } from "@/features/agents/channelAgents";
import {
  channelsQueryKey,
  useCreateChannelMutation,
} from "@/features/channels/hooks";
import { useSendMessageMutation } from "@/features/messages/hooks";
import { addChannelMembers } from "@/shared/api/tauri";
import type { Channel, Identity } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { slugifyPrompt } from "@/features/dev-mode/lib/sessionNaming";
import type {
  DevAgentTarget,
  DevComposerMode,
} from "@/features/dev-mode/lib/useDevComposerModes";

async function ensureAgentInChannel(channelId: string, target: DevAgentTarget) {
  if (target.source === "managed" && target.managedAgent) {
    await attachManagedAgentToChannel(channelId, {
      agent: target.managedAgent,
    });
    return;
  }

  const result = await addChannelMembers({
    channelId,
    pubkeys: [target.pubkey],
    role: "bot",
  });
  const failure = result.errors.find(
    (error) => normalizePubkey(error.pubkey) === normalizePubkey(target.pubkey),
  );
  if (failure) {
    throw new Error(failure.error);
  }
}

export function useDevSessionActions(identity: Identity | undefined) {
  const queryClient = useQueryClient();
  const createChannelMutation = useCreateChannelMutation();
  const sendMessageMutation = useSendMessageMutation(null, identity);

  /**
   * Create the channel for a new session, named and described from the
   * prompt. Creation is separate from the first send so a failure after this
   * point leaves an open, recoverable session instead of a duplicate channel
   * on retry.
   */
  const createSessionChannel = React.useCallback(
    async (prompt: string): Promise<Channel> => {
      const existingNames = new Set(
        (queryClient.getQueryData<Channel[]>(channelsQueryKey) ?? []).map(
          (channel) => channel.name,
        ),
      );

      return createChannelMutation.mutateAsync({
        name: slugifyPrompt(prompt, existingNames),
        channelType: "stream",
        visibility: "open",
        description: prompt.length > 140 ? `${prompt.slice(0, 139)}…` : prompt,
      });
    },
    [createChannelMutation, queryClient],
  );

  /**
   * Send a prompt into a session, optionally as a reply inside an existing
   * thread (`parentEventId`). In an agent mode, the agent is attached first
   * when it is not yet a member (membership must land before the mention or
   * the harness filter drops it) — agents are not limited to a single
   * channel.
   */
  const sendToSession = React.useCallback(
    async (
      channel: Channel,
      prompt: string,
      mode: DevComposerMode,
      parentEventId?: string,
    ) => {
      if (mode.kind === "agent") {
        const isMember = channel.memberPubkeys.some(
          (pubkey) =>
            normalizePubkey(pubkey) === normalizePubkey(mode.target.pubkey),
        );
        if (!isMember) {
          await ensureAgentInChannel(channel.id, mode.target);
        }
      }

      await sendMessageMutation.mutateAsync({
        targetChannel: channel,
        content: prompt,
        mentionPubkeys:
          mode.kind === "agent" ? [mode.target.pubkey] : undefined,
        parentEventId: parentEventId ?? null,
      });
    },
    [sendMessageMutation],
  );

  return {
    createSessionChannel,
    sendToSession,
    isCreatingChannel: createChannelMutation.isPending,
  };
}
