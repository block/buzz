import * as React from "react";
import { toast } from "sonner";

import {
  useAttachManagedAgentToChannelMutation,
  useAvailableAcpRuntimes,
  useCreateChannelManagedAgentMutation,
  useManagedAgentsQuery,
  useStartManagedAgentMutation,
} from "@/features/agents/hooks";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import { useAddChannelMembersMutation } from "@/features/channels/hooks";
import type { UseChannelLinksResult } from "@/features/messages/lib/useChannelLinks";
import type { UseEmojiAutocompleteResult } from "@/features/messages/lib/useEmojiAutocomplete";
import {
  buildOutgoingMessage,
  type ImetaMedia,
  mergeOutgoingTags,
} from "@/features/messages/lib/imetaMediaMarkdown";
import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type { UseRichTextEditorResult } from "@/features/messages/lib/useRichTextEditor";
import type { UseDraftsResult } from "@/features/messages/lib/useDrafts";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import type { AcpRuntime, ChannelType, ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { MENTION_REFERENCE_TAG } from "@/shared/lib/resolveMentionNames";
import { buildCustomEmojiTags } from "@/shared/lib/customEmojiTags";

type PendingNonMemberMentionSend = {
  capturedChannelId: string | null;
  /** Thread context captured at submit time — null for main-timeline sends. */
  capturedThreadContext: {
    parentEventId: string | null;
    threadHeadId: string | null;
  } | null;
  finalContent: string;
  mentionPubkeys: string[];
  nonMemberPubkeys: string[];
  outgoingTags?: string[][];
  readyAgentPubkeys?: string[];
  savedContent: string;
  savedImeta: ImetaMedia[];
  savedSpoileredAttachmentUrls: Set<string>;
  sentDraftKey: string | null | undefined;
};

type SendMessageWithMentionFlowInput = {
  capturedChannelId: string | null;
  /** Thread context captured at submit time — null for main-timeline sends. */
  capturedThreadContext?: {
    parentEventId: string | null;
    threadHeadId: string | null;
  } | null;
  pendingImeta: ImetaMedia[];
  sentDraftKey: string | null | undefined;
  spoileredAttachmentUrls?: ReadonlySet<string>;
  trimmed: string;
};

type UseMentionSendFlowOptions = {
  channelId: string | null;
  channelLinks: Pick<UseChannelLinksResult, "clearChannels">;
  channelType: ChannelType | null;
  contentRef: React.MutableRefObject<string>;
  customEmoji: CustomEmoji[];
  drafts: Pick<UseDraftsResult, "clearDraft">;
  emojiAutocomplete: Pick<UseEmojiAutocompleteResult, "clearEmojis">;
  mentions: UseMentionsResult;
  onSendRef: React.MutableRefObject<
    (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
      channelId?: string | null,
      threadContext?: {
        parentEventId: string | null;
        threadHeadId: string | null;
      } | null,
    ) => Promise<void>
  >;
  richText: Pick<UseRichTextEditorResult, "clearContent" | "setContent">;
  setContent: (content: string) => void;
  setIsEmojiPickerOpen: React.Dispatch<React.SetStateAction<boolean>>;
  setPendingImeta: (pendingImeta: ImetaMedia[]) => void;
  setSpoileredAttachmentUrls?: React.Dispatch<
    React.SetStateAction<Set<string>>
  >;
};

function mergeOutgoingTagsWithReferenceMentions(
  outgoingTags: string[][] | undefined,
  pubkeys: Iterable<string>,
) {
  const normalizedPubkeys = uniqueNormalizedPubkeys(pubkeys);
  if (normalizedPubkeys.length === 0) {
    return outgoingTags;
  }

  return [
    ...(outgoingTags ?? []),
    ...normalizedPubkeys.map((pubkey) => [MENTION_REFERENCE_TAG, pubkey]),
  ];
}

function getErrorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message ? error.message : fallback;
}

function uniqueNormalizedPubkeys(pubkeys: Iterable<string>) {
  return [...new Set([...pubkeys].map(normalizePubkey))].filter(Boolean);
}

function isManagedAgentRunning(agent: ManagedAgent) {
  return agent.status === "running" || agent.status === "deployed";
}

function isProviderBackedAgent(agent: ManagedAgent) {
  return agent.backend.type === "provider";
}

export function useMentionSendFlow({
  channelId,
  channelLinks,
  channelType,
  contentRef,
  customEmoji,
  drafts,
  emojiAutocomplete,
  mentions,
  onSendRef,
  richText,
  setContent,
  setIsEmojiPickerOpen,
  setPendingImeta,
  setSpoileredAttachmentUrls,
}: UseMentionSendFlowOptions) {
  const [pendingNonMemberSend, setPendingNonMemberSend] =
    React.useState<PendingNonMemberMentionSend | null>(null);
  const [nonMemberPromptError, setNonMemberPromptError] = React.useState<
    string | null
  >(null);
  const [isMentionSendPending, setIsMentionSendPending] = React.useState(false);
  const [isCompleteSendPending, setIsCompleteSendPending] =
    React.useState(false);
  const isMentionSendPendingRef = React.useRef(false);
  const isCompleteSendPendingRef = React.useRef(false);
  const previousChannelIdRef = React.useRef(channelId);
  // Tracks the live channel so completeSend can ask "is the user still here?"
  // without being frozen to the compose-time closure.
  const channelIdRef = React.useRef(channelId);
  channelIdRef.current = channelId;

  const addMembersMutation = useAddChannelMembersMutation(channelId);
  const attachAgentMutation = useAttachManagedAgentToChannelMutation(channelId);
  const createPersonaAgentMutation =
    useCreateChannelManagedAgentMutation(channelId);
  const availableRuntimesQuery = useAvailableAcpRuntimes();
  const managedAgentsQuery = useManagedAgentsQuery();
  const startAgentMutation = useStartManagedAgentMutation();

  const getManagedAgentsByPubkey = React.useCallback(async () => {
    const agents =
      managedAgentsQuery.data ??
      (await managedAgentsQuery.refetch()).data ??
      [];

    return new Map(
      agents.map((agent) => [normalizePubkey(agent.pubkey), agent]),
    );
  }, [managedAgentsQuery.data, managedAgentsQuery.refetch]);

  const getAvailableRuntimes = React.useCallback(async (): Promise<
    AcpRuntime[]
  > => {
    const cached = availableRuntimesQuery.data ?? [];
    if (cached.length > 0 || !availableRuntimesQuery.isLoading) {
      return cached;
    }

    const refetched = await availableRuntimesQuery.refetch();
    return (refetched.data ?? []).filter(
      (runtime): runtime is AcpRuntime =>
        runtime.availability === "available" &&
        runtime.command !== null &&
        runtime.binaryPath !== null,
    );
  }, [
    availableRuntimesQuery.data,
    availableRuntimesQuery.isLoading,
    availableRuntimesQuery.refetch,
  ]);

  const ensureManagedAgentMentionsReady = React.useCallback(
    async (mentionPubkeys: string[], capturedChannelId: string) => {
      if (!capturedChannelId || mentionPubkeys.length === 0) {
        return [];
      }

      const managedAgentsByPubkey = await getManagedAgentsByPubkey();
      const errors: string[] = [];

      for (const pubkey of uniqueNormalizedPubkeys(mentionPubkeys)) {
        const agent = managedAgentsByPubkey.get(pubkey);
        if (!agent) {
          continue;
        }

        try {
          if (mentions.memberPubkeys.has(pubkey)) {
            if (isProviderBackedAgent(agent)) {
              if (agent.status !== "deployed") {
                await startAgentMutation.mutateAsync(agent.pubkey);
              }
            } else if (!isManagedAgentRunning(agent)) {
              await startAgentMutation.mutateAsync(agent.pubkey);
            }
          } else {
            await attachAgentMutation.mutateAsync({
              channelId: capturedChannelId,
              agent,
              role: "bot",
            });
          }
        } catch (error) {
          errors.push(
            `${agent.name}: ${getErrorMessage(
              error,
              "Could not prepare agent.",
            )}`,
          );
        }
      }

      return errors;
    },
    [
      attachAgentMutation,
      getManagedAgentsByPubkey,
      mentions.memberPubkeys,
      startAgentMutation,
    ],
  );

  const createMentionedPersonaAgents = React.useCallback(
    async (trimmed: string, capturedChannelId: string) => {
      const personaMentions = mentions.extractMentionPersonas(trimmed);
      if (!capturedChannelId || personaMentions.length === 0) {
        return {
          errors: [] as string[],
          pubkeys: [] as string[],
        };
      }

      const runtimes = await getAvailableRuntimes();
      const defaultRuntime = runtimes[0] ?? null;
      const errors: string[] = [];
      const pubkeys: string[] = [];
      const seenPersonaIds = new Set<string>();

      for (const { displayName, persona } of personaMentions) {
        if (seenPersonaIds.has(persona.id)) {
          continue;
        }
        seenPersonaIds.add(persona.id);

        const { runtime } = resolvePersonaRuntime(
          persona.runtime,
          runtimes,
          defaultRuntime,
        );
        if (!runtime) {
          errors.push(`${displayName}: No agent runtime available.`);
          continue;
        }

        try {
          const result = await createPersonaAgentMutation.mutateAsync({
            channelId: capturedChannelId,
            runtime,
            name: persona.displayName,
            personaId: persona.id,
            systemPrompt: persona.systemPrompt,
            avatarUrl: persona.avatarUrl ?? undefined,
            model: persona.model ?? undefined,
            role: "bot",
            ensureRunning: true,
          });
          const pubkey = normalizePubkey(result.agent.pubkey);
          pubkeys.push(pubkey);
          mentions.registerMentionPubkey(displayName, pubkey, {
            isAgent: true,
          });
        } catch (error) {
          errors.push(
            `${displayName}: ${getErrorMessage(
              error,
              "Could not create agent.",
            )}`,
          );
        }
      }

      return {
        errors,
        pubkeys: uniqueNormalizedPubkeys(pubkeys),
      };
    },
    [
      createPersonaAgentMutation,
      getAvailableRuntimes,
      mentions.extractMentionPersonas,
      mentions.registerMentionPubkey,
    ],
  );

  const clearComposer = React.useCallback(() => {
    setPendingNonMemberSend(null);
    setNonMemberPromptError(null);
    setContent("");
    contentRef.current = "";
    richText.clearContent();
    setPendingImeta([]);
    setSpoileredAttachmentUrls?.(new Set());
    mentions.clearMentions();
    channelLinks.clearChannels();
    emojiAutocomplete.clearEmojis();
    setIsEmojiPickerOpen(false);
  }, [
    channelLinks.clearChannels,
    contentRef,
    emojiAutocomplete.clearEmojis,
    mentions.clearMentions,
    richText.clearContent,
    setContent,
    setIsEmojiPickerOpen,
    setPendingImeta,
    setSpoileredAttachmentUrls,
  ]);

  React.useEffect(() => {
    if (previousChannelIdRef.current === channelId) {
      return;
    }

    previousChannelIdRef.current = channelId;
    setPendingNonMemberSend(null);
    setNonMemberPromptError(null);
  }, [channelId]);

  const completeSend = React.useCallback(
    async (
      draft: PendingNonMemberMentionSend,
      mentionPubkeys: string[],
      outgoingTags = draft.outgoingTags,
    ) => {
      if (isCompleteSendPendingRef.current) {
        return;
      }

      isCompleteSendPendingRef.current = true;
      setIsCompleteSendPending(true);
      try {
        const readyAgentPubkeys = new Set(
          (draft.readyAgentPubkeys ?? []).map(normalizePubkey),
        );
        const agentReadinessErrors = await ensureManagedAgentMentionsReady(
          mentionPubkeys.filter(
            (pubkey) => !readyAgentPubkeys.has(normalizePubkey(pubkey)),
          ),
          draft.capturedChannelId ?? "",
        );
        if (agentReadinessErrors.length > 0) {
          const message =
            agentReadinessErrors.length === 1
              ? `Could not start agent mention: ${agentReadinessErrors[0]}`
              : `Could not start agent mentions: ${agentReadinessErrors.join(
                  "; ",
                )}`;
          setNonMemberPromptError(message);
          toast.error(message);
          return;
        }

        // Only clear the composer if the user has not switched channels since
        // submit. If they have, the composer they see belongs to the new channel
        // and we must not wipe it.
        if (draft.capturedChannelId === channelIdRef.current) {
          clearComposer();
        }

        try {
          await onSendRef.current(
            draft.finalContent,
            mentionPubkeys,
            outgoingTags,
            draft.capturedChannelId,
            draft.capturedThreadContext,
          );
          if (draft.sentDraftKey) {
            drafts.clearDraft(draft.sentDraftKey);
          }
        } catch {
          // Only restore the composer content if the user is still on the
          // channel that originated the send.
          if (draft.capturedChannelId === channelIdRef.current) {
            setContent(draft.savedContent);
            contentRef.current = draft.savedContent;
            richText.setContent(draft.savedContent);
            setPendingImeta(draft.savedImeta);
            setSpoileredAttachmentUrls?.(
              new Set(draft.savedSpoileredAttachmentUrls),
            );
          }
        }
      } finally {
        isCompleteSendPendingRef.current = false;
        setIsCompleteSendPending(false);
      }
    },
    [
      clearComposer,
      contentRef,
      drafts,
      ensureManagedAgentMentionsReady,
      onSendRef,
      richText.setContent,
      setContent,
      setPendingImeta,
      setSpoileredAttachmentUrls,
    ],
  );

  const getNonMemberMentionPubkeys = React.useCallback(
    (pubkeys: string[]) => {
      if (
        channelType === null ||
        channelType === "dm" ||
        !mentions.hasResolvedMembers
      ) {
        return [];
      }

      return uniqueNormalizedPubkeys(pubkeys).filter(
        (pubkey) => !mentions.memberPubkeys.has(pubkey),
      );
    },
    [channelType, mentions.hasResolvedMembers, mentions.memberPubkeys],
  );

  const sendMessageWithMentionFlow = React.useCallback(
    async ({
      capturedChannelId,
      capturedThreadContext = null,
      pendingImeta,
      sentDraftKey,
      spoileredAttachmentUrls = new Set(),
      trimmed,
    }: SendMessageWithMentionFlowInput) => {
      if (isMentionSendPendingRef.current) {
        return;
      }

      isMentionSendPendingRef.current = true;
      setIsMentionSendPending(true);
      try {
        const personaMentionResult = await createMentionedPersonaAgents(
          trimmed,
          capturedChannelId ?? "",
        );
        if (personaMentionResult.errors.length > 0) {
          const message =
            personaMentionResult.errors.length === 1
              ? `Could not create agent mention: ${personaMentionResult.errors[0]}`
              : `Could not create agent mentions: ${personaMentionResult.errors.join(
                  "; ",
                )}`;
          setNonMemberPromptError(message);
          toast.error(message);
          return;
        }

        const createdPersonaAgentPubkeys = personaMentionResult.pubkeys;
        const createdPersonaAgentPubkeySet = new Set(
          createdPersonaAgentPubkeys.map(normalizePubkey),
        );
        const pubkeys = uniqueNormalizedPubkeys([
          ...mentions.extractMentionPubkeys(trimmed),
          ...createdPersonaAgentPubkeys,
        ]);
        const { content: finalContent, mediaTags } = buildOutgoingMessage(
          trimmed,
          pendingImeta,
          spoileredAttachmentUrls,
        );
        const outgoingTags = mergeOutgoingTags(
          mediaTags,
          buildCustomEmojiTags(finalContent, customEmoji),
        );
        const nonMemberPubkeys = getNonMemberMentionPubkeys(pubkeys);
        let promptNonMemberPubkeys = nonMemberPubkeys.filter(
          (pubkey) =>
            !mentions.isManagedAgentPubkey(pubkey) &&
            !createdPersonaAgentPubkeySet.has(normalizePubkey(pubkey)),
        );

        if (promptNonMemberPubkeys.length > 0) {
          try {
            const managedAgentsByPubkey = await getManagedAgentsByPubkey();
            promptNonMemberPubkeys = promptNonMemberPubkeys.filter(
              (pubkey) => !managedAgentsByPubkey.has(normalizePubkey(pubkey)),
            );
          } catch {
            // Keep the hook-based managed-agent filtering even if the query
            // fallback misses; ordinary non-members still get prompted.
          }
        }

        const pendingDraft: PendingNonMemberMentionSend = {
          capturedChannelId,
          capturedThreadContext,
          finalContent,
          mentionPubkeys: pubkeys,
          nonMemberPubkeys: promptNonMemberPubkeys,
          outgoingTags,
          readyAgentPubkeys: createdPersonaAgentPubkeys,
          savedContent: trimmed,
          savedImeta: [...pendingImeta],
          savedSpoileredAttachmentUrls: new Set(spoileredAttachmentUrls),
          sentDraftKey,
        };

        if (promptNonMemberPubkeys.length > 0) {
          setNonMemberPromptError(null);
          setPendingNonMemberSend(pendingDraft);
          return;
        }

        await completeSend(pendingDraft, pubkeys);
      } finally {
        isMentionSendPendingRef.current = false;
        setIsMentionSendPending(false);
      }
    },
    [
      completeSend,
      createMentionedPersonaAgents,
      customEmoji,
      getManagedAgentsByPubkey,
      getNonMemberMentionPubkeys,
      mentions.extractMentionPubkeys,
      mentions.isManagedAgentPubkey,
    ],
  );

  const pendingNonMemberNames = React.useMemo(() => {
    if (!pendingNonMemberSend) return [];

    return pendingNonMemberSend.nonMemberPubkeys.map(
      (pubkey) => mentions.getMentionDisplayName(pubkey) ?? pubkey.slice(0, 8),
    );
  }, [mentions.getMentionDisplayName, pendingNonMemberSend]);

  const handleSendWithoutInviting = React.useCallback(() => {
    if (!pendingNonMemberSend) return;

    const nonMemberPubkeys = new Set(
      pendingNonMemberSend.nonMemberPubkeys.map((pubkey) =>
        normalizePubkey(pubkey),
      ),
    );
    const mentionPubkeys = pendingNonMemberSend.mentionPubkeys.filter(
      (pubkey) => !nonMemberPubkeys.has(normalizePubkey(pubkey)),
    );
    const outgoingTags = mergeOutgoingTagsWithReferenceMentions(
      pendingNonMemberSend.outgoingTags,
      nonMemberPubkeys,
    );
    void completeSend(pendingNonMemberSend, mentionPubkeys, outgoingTags);
  }, [completeSend, pendingNonMemberSend]);

  const handleInviteNonMembers = React.useCallback(() => {
    if (!pendingNonMemberSend) return;

    const invitedPubkeys = new Set(
      pendingNonMemberSend.nonMemberPubkeys.map(normalizePubkey),
    );
    const mentionPubkeys = uniqueNormalizedPubkeys([
      ...pendingNonMemberSend.mentionPubkeys,
      ...pendingNonMemberSend.nonMemberPubkeys,
    ]);
    const outgoingTags = (pendingNonMemberSend.outgoingTags ?? []).filter(
      (tag) =>
        tag[0] !== MENTION_REFERENCE_TAG ||
        !invitedPubkeys.has(normalizePubkey(tag[1] ?? "")),
    );

    setNonMemberPromptError(null);
    void (async () => {
      const managedAgentsByPubkey = await getManagedAgentsByPubkey();
      const peoplePubkeys: string[] = [];
      const relayAgentPubkeys: string[] = [];

      for (const pubkey of uniqueNormalizedPubkeys(
        pendingNonMemberSend.nonMemberPubkeys,
      )) {
        if (managedAgentsByPubkey.has(pubkey)) {
          continue;
        }

        if (mentions.isAgentPubkey(pubkey)) {
          relayAgentPubkeys.push(pubkey);
        } else {
          peoplePubkeys.push(pubkey);
        }
      }

      const errors: string[] = [];
      if (peoplePubkeys.length > 0) {
        const result = await addMembersMutation.mutateAsync({
          channelId: pendingNonMemberSend.capturedChannelId ?? undefined,
          pubkeys: peoplePubkeys,
          role: "member",
        });
        errors.push(...result.errors.map((error) => error.error));
      }

      if (relayAgentPubkeys.length > 0) {
        const result = await addMembersMutation.mutateAsync({
          channelId: pendingNonMemberSend.capturedChannelId ?? undefined,
          pubkeys: relayAgentPubkeys,
          role: "bot",
        });
        errors.push(...result.errors.map((error) => error.error));
      }

      if (errors.length > 0) {
        setNonMemberPromptError(errors.join("; "));
        return;
      }

      await completeSend(
        {
          ...pendingNonMemberSend,
          mentionPubkeys,
          outgoingTags,
        },
        mentionPubkeys,
        outgoingTags,
      );
    })().catch((error) => {
      setNonMemberPromptError(
        error instanceof Error ? error.message : "Could not invite members.",
      );
    });
  }, [
    addMembersMutation,
    completeSend,
    getManagedAgentsByPubkey,
    mentions.isAgentPubkey,
    pendingNonMemberSend,
  ]);

  const dismissNonMemberPrompt = React.useCallback(() => {
    setPendingNonMemberSend(null);
    setNonMemberPromptError(null);
  }, []);

  return {
    dismissNonMemberPrompt,
    isInvitePending:
      isMentionSendPending ||
      isCompleteSendPending ||
      addMembersMutation.isPending ||
      attachAgentMutation.isPending ||
      createPersonaAgentMutation.isPending ||
      startAgentMutation.isPending,
    isPreparingMentionSend:
      isMentionSendPending ||
      isCompleteSendPending ||
      attachAgentMutation.isPending ||
      createPersonaAgentMutation.isPending ||
      startAgentMutation.isPending,
    nonMemberPromptError,
    pendingNonMemberNames,
    pendingNonMemberSend,
    sendMessageWithMentionFlow,
    sendWithoutInviting: handleSendWithoutInviting,
    inviteNonMembers: handleInviteNonMembers,
  };
}
