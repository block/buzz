import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { attachManagedAgentToChannel } from "@/features/agents/channelAgents";
import {
  channelsQueryKey,
  useCreateChannelMutation,
} from "@/features/channels/hooks";
import { useSendMessageMutation } from "@/features/messages/hooks";
import { addChannelMembers } from "@/shared/api/tauri";
import { updateChannel } from "@/shared/api/tauriChannels";
import type { Channel, Identity } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { generateChannelTitle } from "@/features/dev-mode/lib/channelNaming";
import {
  slugifyPrompt,
  uniqueChannelName,
} from "@/features/dev-mode/lib/sessionNaming";
import type {
  DevAgentTarget,
  DevComposerMode,
} from "@/features/dev-mode/lib/useDevComposerModes";

/**
 * Everyone in the channel sees which agent a prompt is directed at: the
 * message text carries a visible `@Name` prefix (matching the standard
 * composer's mention-text convention) unless the user already typed one.
 */
export function withAgentMention(prompt: string, name: string): string {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const alreadyMentioned = new RegExp(
    `(^|\\s)@${escaped}(?=$|[\\s,.;:!?)\\]])`,
    "i",
  ).test(prompt);
  return alreadyMentioned ? prompt : `@${name} ${prompt}`;
}

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

      const channel = await createChannelMutation.mutateAsync({
        name: slugifyPrompt(prompt, existingNames),
        channelType: "stream",
        visibility: "open",
        description: prompt.length > 140 ? `${prompt.slice(0, 139)}…` : prompt,
      });

      // LLM naming is best-effort and never blocks the session: the channel
      // opens under its slug name and is renamed when a title arrives.
      void (async () => {
        const title = await generateChannelTitle(prompt);
        if (!title || title === channel.name) return;
        const currentNames = new Set(
          (queryClient.getQueryData<Channel[]>(channelsQueryKey) ?? [])
            .filter((candidate) => candidate.id !== channel.id)
            .map((candidate) => candidate.name),
        );
        try {
          await updateChannel({
            channelId: channel.id,
            name: uniqueChannelName(title, currentNames),
          });
          await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
        } catch {
          // Rename failing leaves the slug name, which is already valid.
        }
      })();

      return channel;
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
        content:
          mode.kind === "agent"
            ? withAgentMention(prompt, mode.target.name)
            : prompt,
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
