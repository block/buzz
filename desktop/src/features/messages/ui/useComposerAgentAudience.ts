import * as React from "react";

import { useUnaddressedChannelAgentMode } from "@/features/channels/lib/unaddressedChannelAgentMode";
import {
  describeComposerAudienceHint,
  resolveComposerSendAudience,
  type ComposerSendAudienceResult,
} from "@/features/messages/lib/composerSendAudience";
import { getPersistentAgentAudienceScope } from "@/features/messages/lib/persistentAgentAudience";
import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type { UseRichTextEditorResult } from "@/features/messages/lib/useRichTextEditor";
import type { ChannelType } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

import type { usePersistentAgentMentionHydration } from "./usePersistentAgentMentionHydration";

type PersistentHydration = ReturnType<
  typeof usePersistentAgentMentionHydration
>;

export function useComposerAgentAudience({
  audienceThreadRootId,
  channelType,
  editTarget,
  mentions,
  ownerPubkey,
  persistentMentionHydration,
  richText,
}: {
  audienceThreadRootId: string | null;
  channelType: ChannelType | null;
  editTarget: unknown;
  mentions: UseMentionsResult;
  ownerPubkey: string | null | undefined;
  persistentMentionHydration: PersistentHydration;
  richText: UseRichTextEditorResult;
}): {
  composerAudienceHint: string | null;
  audienceGeneration: number;
  audienceRevision: number;
  resolveComposerAudience: (input: {
    explicitMentionPubkeys: string[];
    explicitAgentPubkeys: string[];
    messagePosition: "top-level" | "in-thread";
    threadRootEventId: string | null;
  }) => ComposerSendAudienceResult;
  onSuccessfulExplicitAgentAudience:
    | ((audience: {
        channelId: string;
        expectedGeneration: number;
        expectedRevision: number | null;
        explicitAgentPubkeys: string[];
      }) => void)
    | undefined;
  resolvePostSendContent: PersistentHydration["resolvePostSendContent"];
} {
  const persistentAudience = persistentMentionHydration.audience;
  const { mode: unaddressedMode } = useUnaddressedChannelAgentMode();
  const conversationKind = channelType === "dm" ? "direct" : "channel";

  const channelMemberPubkeyList = React.useMemo(
    () => [...mentions.memberPubkeys],
    [mentions.memberPubkeys],
  );
  const verifiedChannelAgentPubkeys = React.useMemo(
    () => channelMemberPubkeyList.filter((pk) => mentions.isAgentPubkey(pk)),
    [channelMemberPubkeyList, mentions.isAgentPubkey],
  );
  const currentAgentPubkey = React.useMemo(() => {
    if (conversationKind !== "direct") return null;
    const agents = verifiedChannelAgentPubkeys.filter(
      (pk) => pk !== normalizePubkey(ownerPubkey ?? ""),
    );
    return agents[0] ?? null;
  }, [conversationKind, ownerPubkey, verifiedChannelAgentPubkeys]);

  const resolveComposerAudience = React.useCallback(
    ({
      explicitMentionPubkeys,
      explicitAgentPubkeys,
      messagePosition,
      threadRootEventId,
    }: {
      explicitMentionPubkeys: string[];
      explicitAgentPubkeys: string[];
      messagePosition: "top-level" | "in-thread";
      threadRootEventId: string | null;
    }) =>
      resolveComposerSendAudience({
        conversation: conversationKind,
        messagePosition,
        unaddressedMode,
        keepAddressedAgentsActive: persistentAudience.enabled,
        explicitMentionPubkeys,
        explicitAgentPubkeys,
        currentAgentPubkey,
        channelMemberPubkeys: channelMemberPubkeyList,
        verifiedChannelAgentPubkeys,
        persistentThreadAudience: [...persistentAudience.pubkeys],
        threadRootEventId,
        recipientLoadError:
          !mentions.hasResolvedMembers && conversationKind === "channel",
      }),
    [
      channelMemberPubkeyList,
      conversationKind,
      currentAgentPubkey,
      mentions.hasResolvedMembers,
      persistentAudience.enabled,
      persistentAudience.pubkeys,
      unaddressedMode,
      verifiedChannelAgentPubkeys,
    ],
  );

  const composerAudienceHint = React.useMemo(() => {
    if (editTarget != null || conversationKind === "direct") return null;
    const text = richText.getPlainTextAndCursor().text;
    const explicitMentionPubkeys = mentions.extractMentionPubkeys(text);
    const explicitAgentPubkeys = explicitMentionPubkeys.filter((pk) =>
      mentions.isAgentPubkey(pk),
    );
    const decision = resolveComposerAudience({
      explicitMentionPubkeys,
      explicitAgentPubkeys,
      messagePosition: audienceThreadRootId ? "in-thread" : "top-level",
      threadRootEventId: audienceThreadRootId,
    });
    return describeComposerAudienceHint({
      conversation: conversationKind,
      unaddressedMode,
      explicitAgentCount: explicitAgentPubkeys.length,
      implicitAgentCount:
        explicitAgentPubkeys.length > 0
          ? 0
          : decision.agentAudiencePubkeys.length,
      retainDraft: decision.retainDraft,
    });
  }, [
    audienceThreadRootId,
    conversationKind,
    editTarget,
    mentions,
    resolveComposerAudience,
    richText,
    unaddressedMode,
  ]);

  const onSuccessfulExplicitAgentAudience =
    persistentAudience.enabled && ownerPubkey
      ? ({
          channelId: successfulChannelId,
          ...promotion
        }: {
          channelId: string;
          expectedGeneration: number;
          expectedRevision: number | null;
          explicitAgentPubkeys: string[];
        }) => {
          const scope = getPersistentAgentAudienceScope({
            ownerPubkey,
            channelId: successfulChannelId,
            threadRootId: audienceThreadRootId,
          });
          persistentAudience.promotePubkeys({ ...promotion, scope });
        }
      : undefined;

  return {
    composerAudienceHint,
    audienceGeneration: persistentAudience.generation,
    audienceRevision: persistentAudience.revision,
    resolveComposerAudience,
    onSuccessfulExplicitAgentAudience,
    resolvePostSendContent: persistentMentionHydration.resolvePostSendContent,
  };
}
