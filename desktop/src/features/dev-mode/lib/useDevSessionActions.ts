import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { attachManagedAgentToChannel } from "@/features/agents/channelAgents";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  channelsQueryKey,
  useCreateChannelMutation,
} from "@/features/channels/hooks";
import { useSendMessageMutation } from "@/features/messages/hooks";
import {
  buildOutgoingMessage,
  type ImetaMedia,
} from "@/features/messages/lib/imetaMediaMarkdown";
import { addChannelMembers, getCanvas, setCanvas } from "@/shared/api/tauri";
import { updateChannel } from "@/shared/api/tauriChannels";
import type { Channel, Identity } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { generateChannelTitle } from "@/features/dev-mode/lib/channelNaming";
import type { MentionRecord } from "@/features/dev-mode/lib/mentionRecords";
import { uniqueChannelName } from "@/features/dev-mode/lib/sessionNaming";
import {
  appendSubChannelToParentCanvas,
  parseSubChannelName,
  subChannelAnnouncement,
  subChannelCanvasDoc,
  subChannelName,
} from "@/features/dev-mode/lib/subChannels";
import type {
  DevAgentTarget,
  DevComposerMode,
} from "@/features/dev-mode/lib/useDevComposerModes";

/**
 * Everyone in the channel sees which agent a prompt is directed at: the
 * message text carries a `@Name` prefix (matching the standard composer's
 * mention-text convention) unless the user already typed one. Dev mode lifts
 * the prefix out of the body at render time and shows it as a "to Name" line
 * under the author; the standard UI still shows the literal mention text.
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
  // "Already a member" satisfies the goal — the membership snapshot this
  // check ran against can be stale right after a prior send added the agent.
  if (failure && !/already a member/i.test(failure.error)) {
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

  const cachedChannels = React.useCallback(
    () => queryClient.getQueryData<Channel[]>(channelsQueryKey) ?? [],
    [queryClient],
  );

  /** The parent channel when `channel` is a `parent--sub`, else null. */
  const findParentChannel = React.useCallback(
    (channel: Channel): Channel | null => {
      const parsed = parseSubChannelName(channel.name);
      if (!parsed) return null;
      return (
        cachedChannels().find(
          (candidate) =>
            candidate.name === parsed.parentName && candidate.id !== channel.id,
        ) ?? null
      );
    },
    [cachedChannels],
  );

  /**
   * Spawn a sub-channel of `parent` for a task (mirrors `buzz channels
   * create --parent`): the child inherits the parent's type/visibility and
   * is named `parent--<slug>`. The channel opens immediately under a
   * placeholder slug; naming, the parent announcement, and the
   * relationship canvases follow best-effort in the background —
   * announcement and canvases wait for the rename so they reference the
   * final name.
   */
  const createSubChannel = React.useCallback(
    async (
      parent: Channel,
      prompt: string,
      mode?: DevComposerMode,
    ): Promise<Channel> => {
      const existingNames = new Set(
        cachedChannels().map((channel) => channel.name),
      );
      const channel = await createChannelMutation.mutateAsync({
        name: uniqueChannelName(
          subChannelName(parent.name, "new-sub"),
          existingNames,
        ),
        // Inherit the parent's type/visibility; DMs can't have subs, so
        // anything non-forum becomes a stream (dev sessions are streams).
        channelType: parent.channelType === "forum" ? "forum" : "stream",
        visibility: parent.visibility,
        description: prompt.length > 140 ? `${prompt.slice(0, 139)}…` : prompt,
      });

      void (async () => {
        let finalName = channel.name;
        const title = await generateChannelTitle(
          prompt,
          namingAgentPubkey(mode),
        );
        if (title) {
          const currentNames = new Set(
            cachedChannels()
              .filter((candidate) => candidate.id !== channel.id)
              .map((candidate) => candidate.name),
          );
          const candidate = uniqueChannelName(
            subChannelName(parent.name, title),
            currentNames,
          );
          try {
            await updateChannel({ channelId: channel.id, name: candidate });
            finalName = candidate;
            await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
          } catch (error) {
            console.warn(
              `dev-mode: sub-channel rename failed for ${channel.id}`,
              error,
            );
          }
        }

        try {
          const announcement = await sendMessageMutation.mutateAsync({
            targetChannel: parent,
            content: subChannelAnnouncement(finalName),
          });
          await setCanvas({
            channelId: channel.id,
            content: subChannelCanvasDoc({
              parentName: parent.name,
              parentId: parent.id,
              announcementEventId: announcement.id,
              task: prompt,
            }),
          });
          const parentCanvas = await getCanvas(parent.id);
          await setCanvas({
            channelId: parent.id,
            content: appendSubChannelToParentCanvas(
              parentCanvas.content,
              finalName,
              prompt,
            ),
          });
        } catch (error) {
          console.warn(
            `dev-mode: sub-channel announcement/canvas setup failed for ${channel.id}`,
            error,
          );
        }
      })();

      return channel;
    },
    [
      cachedChannels,
      createChannelMutation,
      namingAgentPubkey,
      queryClient,
      sendMessageMutation,
    ],
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
      mentions: MentionRecord[] = [],
      media: ImetaMedia[] = [],
    ) => {
      // Sub-channel invariant (client-side; the relay knows nothing about
      // sub-channels): everyone in `parent--sub` must belong to `parent`.
      const parentChannel = findParentChannel(channel);
      const parentMemberPubkeys = parentChannel
        ? new Set(
            parentChannel.memberPubkeys.map((pubkey) =>
              normalizePubkey(pubkey),
            ),
          )
        : null;

      if (mode.kind === "agent") {
        const agentPubkey = normalizePubkey(mode.target.pubkey);
        if (parentChannel && !parentMemberPubkeys?.has(agentPubkey)) {
          await ensureAgentInChannel(parentChannel.id, mode.target);
          parentMemberPubkeys?.add(agentPubkey);
        }
        const isMember = channel.memberPubkeys.some(
          (pubkey) => normalizePubkey(pubkey) === agentPubkey,
        );
        if (!isMember) {
          await ensureAgentInChannel(channel.id, mode.target);
        }
      }

      // Tagging someone pulls them into the channel (mirroring how the
      // targeted agent is auto-attached); best-effort so a failed add never
      // blocks the send — the p tag still goes out.
      const memberPubkeys = new Set(
        channel.memberPubkeys.map((pubkey) => normalizePubkey(pubkey)),
      );
      if (mode.kind === "agent") {
        memberPubkeys.add(normalizePubkey(mode.target.pubkey));
      }
      let nonMembers = mentions.filter(
        (mention) => !memberPubkeys.has(normalizePubkey(mention.pubkey)),
      );
      if (parentMemberPubkeys) {
        const outsiders = nonMembers.filter(
          (mention) =>
            !parentMemberPubkeys.has(normalizePubkey(mention.pubkey)),
        );
        if (outsiders.length > 0) {
          console.warn(
            `dev-mode: skipping auto-add of ${outsiders.length} mention(s) not in the parent channel of ${channel.name}`,
          );
          nonMembers = nonMembers.filter((mention) =>
            parentMemberPubkeys.has(normalizePubkey(mention.pubkey)),
          );
        }
      }
      for (const role of ["member", "bot"] as const) {
        const pubkeys = nonMembers
          .filter((mention) => (role === "bot") === mention.isAgent)
          .map((mention) => mention.pubkey);
        if (pubkeys.length === 0) continue;
        try {
          await addChannelMembers({ channelId: channel.id, pubkeys, role });
        } catch (addError) {
          console.warn(
            `dev-mode: failed to add mentioned ${role}s to ${channel.id}:`,
            addError,
          );
        }
      }

      const mentionPubkeys = [
        ...new Set([
          ...(mode.kind === "agent" ? [mode.target.pubkey] : []),
          ...mentions.map((mention) => mention.pubkey),
        ]),
      ];

      // Attachments append `![image|video](url)` lines to the body (the
      // renderer keys on URLs literally present in the content) and ride as
      // NIP-92 imeta tags — the same wire shape the standard composer sends.
      const { content, mediaTags } = buildOutgoingMessage(
        mode.kind === "agent"
          ? withAgentMention(prompt, mode.target.name)
          : prompt,
        media,
      );

      return await sendMessageMutation.mutateAsync({
        targetChannel: channel,
        content,
        mentionPubkeys: mentionPubkeys.length > 0 ? mentionPubkeys : undefined,
        parentEventId: parentEventId ?? null,
        mediaTags,
      });
    },
    [findParentChannel, sendMessageMutation],
  );

  return {
    createSessionChannel,
    createSubChannel,
    sendToSession,
    isCreatingChannel: createChannelMutation.isPending,
  };
}
