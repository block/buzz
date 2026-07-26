import * as React from "react";

import { useAgentWorking } from "@/features/agents/agentWorkingSignal";
import {
  HarnessModeScreen,
  type HarnessParticipant,
  type HarnessThreadMessage,
} from "@/features/agents/ui/HarnessModeScreen";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";
import type { TimelineMessage } from "@/features/messages/types";
import { usePresenceQuery } from "@/features/presence/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ManagedAgentSessionPanel } from "@/features/agents/ui/ManagedAgentSessionPanel";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";

type HarnessModeViewProps = {
  agent: React.ComponentProps<typeof ManagedAgentSessionPanel>["agent"];
  canCancelTurn: boolean;
  channelId: string | null;
  channelName: string | null;
  currentUserPubkey: string | null;
  onCancelTurn?: () => void;
  onExit: () => void;
  composerDisabled?: boolean;
  isSending?: boolean;
  onSend?: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    channelId?: string | null,
  ) => Promise<void>;
  /** Messages of the originating thread (head first), for the history rail. */
  threadMessages?: readonly TimelineMessage[];
  /**
   * The agent's own published replies in this thread. Folded into the centre
   * transcript so the final answer appears there, not just the tool call that
   * produced it.
   */
  agentMessages?: readonly TimelineMessage[];
  /** Pubkeys currently typing in this channel/thread. */
  typingPubkeys?: readonly string[];
  profiles?: UserProfileLookup;
};

/**
 * Data container for the full-screen harness view.
 *
 * Keeps every query and derivation out of [`HarnessModeScreen`] so the screen
 * stays presentational (and screenshot-testable with fixture props).
 */
export function HarnessModeView({
  agent,
  canCancelTurn,
  channelId,
  channelName,
  currentUserPubkey,
  onCancelTurn,
  onExit,
  composerDisabled,
  isSending,
  onSend,
  threadMessages,
  agentMessages,
  typingPubkeys,
  profiles,
}: HarnessModeViewProps) {
  const membersQuery = useChannelMembersQuery(channelId);
  const members = membersQuery.data;

  // Agents are members too, but the roster answers "who is watching this run",
  // so the agent doing the work is not one of its own spectators.
  const humanMembers = React.useMemo(
    () => (members ?? []).filter((member) => !member.isAgent),
    [members],
  );

  const memberPubkeys = React.useMemo(
    () => humanMembers.map((member) => member.pubkey),
    [humanMembers],
  );

  const presenceQuery = usePresenceQuery(memberPubkeys, {
    enabled: memberPubkeys.length > 0,
  });
  const presence = presenceQuery.data;

  const participants = React.useMemo<HarnessParticipant[]>(() => {
    const selfPubkey = currentUserPubkey
      ? normalizePubkey(currentUserPubkey)
      : null;

    return humanMembers
      .map((member) => {
        const normalized = normalizePubkey(member.pubkey);
        const profile = profiles?.[normalized];
        return {
          pubkey: member.pubkey,
          displayName:
            profile?.displayName ||
            member.displayName ||
            truncatePubkey(normalized ?? member.pubkey),
          avatarUrl: profile?.avatarUrl ?? null,
          // Carry the real tri-state through — "away" is a distinct presence the
          // shared PresenceDot already renders, so collapsing it to a boolean
          // would show idle teammates as gone.
          status: presence?.[normalized] ?? "offline",
          isSelf: selfPubkey !== null && normalized === selfPubkey,
        };
      })
      .sort((a, b) => {
        const aPresent = a.status !== "offline";
        const bPresent = b.status !== "offline";
        if (aPresent !== bPresent) {
          return aPresent ? -1 : 1;
        }
        return a.displayName.localeCompare(b.displayName);
      });
  }, [currentUserPubkey, humanMembers, presence, profiles]);

  const working = useAgentWorking(agent.pubkey, channelId);

  // The agent's published replies, shaped as assistant transcript rows. The
  // observer stream carries reasoning and tool calls but never the answer text
  // (replies go out through the `buzz` CLI), so without this the transcript
  // shows the work and omits the conclusion.
  const agentReplyItems = React.useMemo<TranscriptItem[]>(() => {
    if (!agentMessages || agentMessages.length === 0) {
      return [];
    }
    return agentMessages.map((message) => ({
      id: `reply:${message.id}`,
      type: "message" as const,
      renderClass: "message" as const,
      role: "assistant" as const,
      title: message.author,
      text: message.body,
      timestamp: new Date(message.createdAt * 1000).toISOString(),
      messageId: message.id,
      authorPubkey: message.pubkey ?? null,
      channelId,
    }));
  }, [agentMessages, channelId]);

  // The agent's own replies plus every human prompt in this thread, as
  // transcript rows.
  //
  // Prompts have to be injected because the ACP stream cannot be relied on to
  // emit one. A goose-native steer writes a `steer:` row, but Claude Code's
  // adapter has no such frame — the harness falls back to cancel+merge, so a
  // mid-turn message is folded into a merged re-prompt and never gets a row of
  // its own. Injecting here (deduped by `messageId`, so the stream's own row
  // wins when it exists) puts the message in the chat in timestamp order, with
  // the answer below it.
  const injectedItems = React.useMemo<TranscriptItem[]>(() => {
    const prompts: TranscriptItem[] = (threadMessages ?? []).map((message) => ({
      id: `prompt:${message.id}`,
      type: "message" as const,
      renderClass: "message" as const,
      role: "user" as const,
      title: message.author,
      text: message.body,
      timestamp: new Date(message.createdAt * 1000).toISOString(),
      messageId: message.id,
      authorPubkey: message.pubkey ?? null,
      channelId,
    }));
    return [...agentReplyItems, ...prompts];
  }, [agentReplyItems, channelId, threadMessages]);

  // Thread rail rows. `isSelf` drives the unread badge, which must ignore your
  // own sends.
  const threadHistory = React.useMemo<
    HarnessThreadMessage[] | undefined
  >(() => {
    if (!threadMessages) {
      return undefined;
    }
    const selfPubkey = currentUserPubkey
      ? normalizePubkey(currentUserPubkey)
      : null;
    return threadMessages.map((message) => ({
      id: message.id,
      author: message.author,
      avatarUrl: message.avatarUrl ?? null,
      body: message.body,
      time: message.time,
      createdAt: message.createdAt,
      authorPubkey: message.pubkey ?? null,
      isSelf:
        selfPubkey !== null &&
        normalizePubkey(message.pubkey ?? "") === selfPubkey,
    }));
  }, [currentUserPubkey, threadMessages]);

  const typingParticipants = React.useMemo<HarnessParticipant[]>(() => {
    if (!typingPubkeys || typingPubkeys.length === 0) {
      return [];
    }
    const typing = new Set(
      typingPubkeys.map((pubkey) => normalizePubkey(pubkey)),
    );
    return participants.filter((participant) =>
      typing.has(normalizePubkey(participant.pubkey)),
    );
  }, [participants, typingPubkeys]);

  // Real thread scope: the ids of this thread's messages. The panel resolves
  // them to turn ids, so two threads in the same channel stay independent —
  // which a timestamp window could never do (the older thread's window always
  // swallows the newer thread's frames).
  const threadMessageIds = React.useMemo(() => {
    const ids = new Set<string>();
    for (const message of threadMessages ?? []) {
      ids.add(message.id);
    }
    for (const message of agentMessages ?? []) {
      ids.add(message.id);
    }
    return ids;
  }, [agentMessages, threadMessages]);

  return (
    <HarnessModeScreen
      agent={agent}
      canCancelTurn={canCancelTurn}
      channelId={channelId}
      channelName={channelName}
      composerDisabled={composerDisabled}
      isSending={isSending}
      isWorking={working.working}
      onCancelTurn={onCancelTurn}
      onExit={onExit}
      onSend={onSend}
      participants={participants}
      profiles={profiles}
      extraTranscriptItems={injectedItems}
      threadMessageIds={threadMessageIds}
      threadMessages={threadHistory}
      typingParticipants={typingParticipants}
    />
  );
}
