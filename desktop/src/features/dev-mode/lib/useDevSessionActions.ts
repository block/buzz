import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { attachManagedAgentToChannel } from "@/features/agents/channelAgents";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
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
import { uniqueChannelName } from "@/features/dev-mode/lib/sessionNaming";
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
  const managedAgentsQuery = useManagedAgentsQuery();
  const managedAgents = managedAgentsQuery.data;

  /**
   * The managed agent whose harness runs the one-shot naming completion:
   * the agent tagged in the composer when it is a managed one, otherwise any
   * configured managed agent. Relay agents and plain chat can't run local
   * completions, so they borrow the first managed agent's harness.
   */
  const namingAgentPubkey = React.useCallback(
    (mode: DevComposerMode | undefined): string | null => {
      if (mode?.kind === "agent" && mode.target.source === "managed") {
        return mode.target.pubkey;
      }
      return managedAgents?.[0]?.pubkey ?? null;
    },
    [managedAgents],
  );

  /**
   * Create the channel for a new session, named and described from the
   * prompt. Creation is separate from the first send so a failure after this
   * point leaves an open, recoverable session instead of a duplicate channel
   * on retry.
   */
  const createSessionChannel = React.useCallback(
    async (prompt: string, mode?: DevComposerMode): Promise<Channel> => {
      const existingNames = new Set(
        (queryClient.getQueryData<Channel[]>(channelsQueryKey) ?? []).map(
          (channel) => channel.name,
        ),
      );

      // Neutral placeholder, never a prompt slug: the name is replaced by an
      // agent-generated title, and a lingering "new-session" makes a naming
      // failure visible instead of masquerading as a generated title.
      const channel = await createChannelMutation.mutateAsync({
        name: uniqueChannelName("new-session", existingNames),
        channelType: "stream",
        visibility: "open",
        description: prompt.length > 140 ? `${prompt.slice(0, 139)}…` : prompt,
      });

      // LLM naming is best-effort and never blocks the session: the channel
      // opens under its placeholder name and is renamed when a title arrives.
      void (async () => {
        const title = await generateChannelTitle(
          prompt,
          namingAgentPubkey(mode),
        );
        if (!title) {
          console.warn(
            `dev-mode: channel naming failed for ${channel.id}; keeping placeholder`,
          );
          return;
        }
        if (title === channel.name) return;
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
        } catch (error) {
          // Rename failing leaves the placeholder name, which is still valid.
          console.warn(
            `dev-mode: channel rename failed for ${channel.id}`,
            error,
          );
        }
      })();

      return channel;
    },
    [createChannelMutation, namingAgentPubkey, queryClient],
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

      return await sendMessageMutation.mutateAsync({
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
