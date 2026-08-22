import * as React from "react";
import { toast } from "sonner";
import {
  type CreateChannelManagedAgentInput,
  useAttachManagedAgentToChannelMutation,
  useAvailableAcpRuntimes,
  useCreateChannelManagedAgentMutation,
  useManagedAgentsQuery,
  useProvisionChannelManagedAgentMutation,
  useStartManagedAgentMutation,
} from "@/features/agents/hooks";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import { useAddChannelMembersMutation } from "@/features/channels/hooks";
import { useCanAddChannelMembers } from "@/features/channels/useCanAddChannelMembers";
import { PRIVATE_CHANNEL_ADD_DENIED_MESSAGE } from "@/features/channels/lib/channelMemberAdmission";
import { dmThreadAgentMentionError } from "@/features/messages/lib/dmThreadAgentMentionError";
import {
  prepareBackgroundMediaUpload,
  saveQueuedAttachmentsForDraft,
  type QueuedMediaAttachment,
} from "@/features/messages/lib/backgroundMediaUploadStore";
import type { UseChannelLinksResult } from "@/features/messages/lib/useChannelLinks";
import type { UseEmojiAutocompleteResult } from "@/features/messages/lib/useEmojiAutocomplete";
import {
  buildOutgoingMessage,
  type ImetaMedia,
} from "@/features/messages/lib/imetaMediaMarkdown";
import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type { UseRichTextEditorResult } from "@/features/messages/lib/useRichTextEditor";
import type { UseDraftsResult } from "@/features/messages/lib/useDrafts";
import { useActivePreparedLinkPreviews } from "./useActivePreparedLinkPreviews";
import { invokeTauri } from "@/shared/api/tauri";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import type { AcpRuntime, ChannelType, ManagedAgent } from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { buildCustomEmojiTags } from "@/shared/lib/customEmojiTags";
import {
  getErrorMessage,
  isManagedAgentRunning,
  isProviderBackedAgent,
  mergeMentionRecipients,
  MENTION_REFERENCE_TAG,
  mergeOutgoingTagsWithReferenceMentions,
  type PendingNonMemberMentionSend,
  type SendMessageWithMentionFlowInput,
  resolvePreviewTags,
  uniqueNormalizedPubkeys,
} from "./useMentionSendFlow.helpers";
import { buildAgentAddressMentionTags } from "@/features/messages/lib/agentAddressMention.mjs";
type UseMentionSendFlowOptions = {
  channelId: string | null;
  channelLinks: Pick<UseChannelLinksResult, "clearChannels">;
  channelType: ChannelType | null;
  contentRef: React.MutableRefObject<string>;
  customEmoji: CustomEmoji[];
  drafts: Pick<UseDraftsResult, "loadDraft" | "markDraftSent" | "persistDraft">;
  emojiAutocomplete: Pick<UseEmojiAutocompleteResult, "clearEmojis">;
  mentions: UseMentionsResult;
  onPrepareSendChannel?: (pubkeys?: string[]) => Promise<string | null>;
  onAddressedAgentsSendStarted?: (pubkeys: readonly string[]) => void;
  onAddressedAgentsSendFailed?: (pubkeys: readonly string[]) => void;
  onInlineAgentMentionsSent?: (promotion: {
    expectedRevision: number;
    pubkeys: readonly string[];
  }) => void;
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
      forceRest?: boolean,
    ) => Promise<void>
  >;
  richText: Pick<UseRichTextEditorResult, "clearContent" | "setContent">;
  setContent: (content: string) => void;
  setIsEmojiPickerOpen: React.Dispatch<React.SetStateAction<boolean>>;
  setPendingImeta: (pendingImeta: ImetaMedia[]) => void;
  hasUnsavedMedia: () => boolean;
  clearQueuedAttachments: () => void;
  restoreQueuedAttachments: (attachments: QueuedMediaAttachment[]) => void;
  setSpoileredAttachmentUrls?: React.Dispatch<
    React.SetStateAction<Set<string>>
  >;
};
export function useMentionSendFlow({
  channelId,
  channelLinks,
  channelType,
  contentRef,
  customEmoji,
  drafts,
  emojiAutocomplete,
  mentions,
  onPrepareSendChannel,
  onAddressedAgentsSendStarted,
  onAddressedAgentsSendFailed,
  onInlineAgentMentionsSent,
  onSendRef,
  richText,
  setContent,
  setIsEmojiPickerOpen,
  setPendingImeta,
  hasUnsavedMedia,
  clearQueuedAttachments,
  restoreQueuedAttachments,
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
  const isMountedRef = React.useRef(false);
  const activePreparedLinkPreviews = useActivePreparedLinkPreviews();
  const previousChannelIdRef = React.useRef(channelId);
  const channelIdRef = React.useRef(channelId);
  channelIdRef.current = channelId;
  React.useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
    };
  }, []);
  const addMembersMutation = useAddChannelMembersMutation(channelId);
  const canInviteNonMembers = useCanAddChannelMembers(channelId);
  const attachAgentMutation = useAttachManagedAgentToChannelMutation(channelId);
  const createPersonaAgentMutation =
    useCreateChannelManagedAgentMutation(channelId);
  const provisionPersonaAgentMutation =
    useProvisionChannelManagedAgentMutation(channelId);
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
    async (
      mentionPubkeys: string[],
      capturedChannelId: string,
      preparedParticipantPubkeys: string[] = [],
      preparedManagedAgents: ManagedAgent[] = [],
    ) => {
      if (!capturedChannelId || mentionPubkeys.length === 0) {
        return {
          errors: [] as string[],
          pubkeys: [] as string[],
        };
      }
      const managedAgentsByPubkey = await getManagedAgentsByPubkey();
      for (const agent of preparedManagedAgents) {
        managedAgentsByPubkey.set(normalizePubkey(agent.pubkey), agent);
      }
      const participantPubkeys = new Set([
        ...mentions.memberPubkeys,
        ...preparedParticipantPubkeys.map(normalizePubkey),
      ]);
      const errors: string[] = [];
      const pubkeys: string[] = [];
      for (const pubkey of uniqueNormalizedPubkeys(mentionPubkeys)) {
        const agent = managedAgentsByPubkey.get(pubkey);
        if (!agent) {
          continue;
        }
        try {
          if (participantPubkeys.has(pubkey)) {
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
          pubkeys.push(pubkey);
        } catch (error) {
          errors.push(
            `${agent.name}: ${getErrorMessage(
              error,
              "Could not prepare agent.",
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
          agents: [] as ManagedAgent[],
          pubkeys: [] as string[],
        };
      }
      const runtimes = await getAvailableRuntimes();
      const defaultRuntime = runtimes[0] ?? null;
      const errors: string[] = [];
      const agents: ManagedAgent[] = [];
      const pubkeys: string[] = [];
      const seenPersonaIds = new Set<string>();
      const shouldProvisionForDm =
        channelType === "dm" && Boolean(onPrepareSendChannel);
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
          const input: CreateChannelManagedAgentInput & {
            channelId: string;
          } = {
            channelId: capturedChannelId,
            runtime,
            name: persona.displayName,
            personaId: persona.id,
            systemPrompt: persona.systemPrompt,
            avatarUrl: persona.avatarUrl ?? undefined,
            model: persona.model ?? undefined,
            role: "bot",
            ensureRunning: true,
          };
          const result = shouldProvisionForDm
            ? await provisionPersonaAgentMutation.mutateAsync(input)
            : await createPersonaAgentMutation.mutateAsync(input);
          const pubkey = normalizePubkey(result.agent.pubkey);
          agents.push(result.agent);
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
        agents,
        errors,
        pubkeys: uniqueNormalizedPubkeys(pubkeys),
      };
    },
    [
      createPersonaAgentMutation,
      channelType,
      getAvailableRuntimes,
      mentions.extractMentionPersonas,
      mentions.registerMentionPubkey,
      onPrepareSendChannel,
      provisionPersonaAgentMutation,
    ],
  );
  const clearComposer = React.useCallback(() => {
    setPendingNonMemberSend(null);
    setNonMemberPromptError(null);
    setContent("");
    contentRef.current = "";
    richText.clearContent();
    setPendingImeta([]);
    clearQueuedAttachments();
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
    clearQueuedAttachments,
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
      const sendSignal = draft.preparedLinkPreviews?.signal;
      const isSendCancelled = () => sendSignal?.aborted === true;
      if (isSendCancelled()) return draft.preparedLinkPreviews?.release();
      isCompleteSendPendingRef.current = true;
      setIsCompleteSendPending(true);
      const preparedUpload =
        draft.queuedAttachments.length > 0
          ? prepareBackgroundMediaUpload(draft.queuedAttachments)
          : null;
      const persistPreflightDraft = () => {
        if (isSendCancelled() || !draft.recoveryDraftKey) return;
        drafts.persistDraft(
          draft.recoveryDraftKey,
          draft.savedContent,
          draft.capturedChannelId ?? draft.recoveryDraftKey,
          draft.savedImeta,
          [...draft.savedSpoileredAttachmentUrls],
          draft.savedMentionRefs,
        );
        saveQueuedAttachmentsForDraft(
          draft.recoveryDraftKey,
          draft.queuedAttachments,
        );
      };
      const persistCanceledDraft = () => {
        if (isSendCancelled() || !draft.recoveryDraftKey) return;
        const existing = drafts.loadDraft(draft.recoveryDraftKey);
        if (
          existing &&
          (existing.content !== draft.savedContent ||
            existing.channelId !==
              (draft.capturedChannelId ?? draft.recoveryDraftKey) ||
            JSON.stringify(existing.pendingImeta) !==
              JSON.stringify(draft.savedImeta) ||
            JSON.stringify(existing.spoileredAttachmentUrls) !==
              JSON.stringify([...draft.savedSpoileredAttachmentUrls]))
        ) {
          return;
        }
        drafts.persistDraft(
          draft.recoveryDraftKey,
          draft.savedContent,
          draft.capturedChannelId ?? draft.recoveryDraftKey,
          draft.savedImeta,
          [...draft.savedSpoileredAttachmentUrls],
          draft.savedMentionRefs,
        );
      };
      let composerCleared = false;
      const restoreComposerAfterFailure = () => {
        if (!composerCleared) return;
        composerCleared = false;
        persistCanceledDraft();
        const canAnimateCurrentComposer =
          isMountedRef.current &&
          (draft.capturedChannelId === channelIdRef.current ||
            channelIdRef.current === null);
        if (
          canAnimateCurrentComposer &&
          draft.addressedAgentPubkeys.length > 0
        ) {
          onAddressedAgentsSendFailed?.(draft.addressedAgentPubkeys);
        }
        const canRestoreCurrentComposer =
          canAnimateCurrentComposer &&
          contentRef.current.trim().length === 0 &&
          !hasUnsavedMedia();
        if (!canRestoreCurrentComposer && draft.recoveryDraftKey) {
          saveQueuedAttachmentsForDraft(
            draft.recoveryDraftKey,
            draft.queuedAttachments,
          );
        }
        if (!canRestoreCurrentComposer) {
          return;
        }
        setContent(draft.savedContent);
        contentRef.current = draft.savedContent;
        richText.setContent(draft.savedContent);
        setPendingImeta(draft.savedImeta);
        restoreQueuedAttachments(draft.queuedAttachments);
        mentions.restoreDraftMentionRefs(draft.savedMentionRefs);
        setSpoileredAttachmentUrls?.(
          new Set(draft.savedSpoileredAttachmentUrls),
        );
      };
      if (
        draft.capturedChannelId === channelIdRef.current ||
        channelIdRef.current === null
      ) {
        if (draft.addressedAgentPubkeys.length > 0) {
          onAddressedAgentsSendStarted?.(draft.addressedAgentPubkeys);
        }
        clearComposer();
        composerCleared = true;
      }
      let uploadStarted = false;
      try {
        const admittedMentionPubkeys = uniqueNormalizedPubkeys(
          await mentions.revalidateMentionPubkeys(mentionPubkeys),
        );
        if (isSendCancelled()) return restoreComposerAfterFailure();
        if (!isMountedRef.current) return persistPreflightDraft();
        const admittedMentionPubkeySet = new Set(admittedMentionPubkeys);
        const readyAgentPubkeys = new Set(
          uniqueNormalizedPubkeys(draft.readyAgentPubkeys ?? []).filter(
            (pubkey) => admittedMentionPubkeySet.has(pubkey),
          ),
        );
        const managedAgentsByPubkey = await getManagedAgentsByPubkey();
        if (isSendCancelled()) return restoreComposerAfterFailure();
        if (!isMountedRef.current) {
          persistPreflightDraft();
          return;
        }
        for (const agent of draft.preparedManagedAgents ?? []) {
          managedAgentsByPubkey.set(normalizePubkey(agent.pubkey), agent);
        }
        const normalizedMentionPubkeys = admittedMentionPubkeys;
        const managedMentionPubkeys = normalizedMentionPubkeys.filter(
          (pubkey) => managedAgentsByPubkey.has(pubkey),
        );
        const agentMentionPubkeys = uniqueNormalizedPubkeys([
          ...managedMentionPubkeys,
          ...normalizedMentionPubkeys.filter(mentions.isAgentPubkey),
        ]);
        const preparedAgentPubkeys = uniqueNormalizedPubkeys([
          ...readyAgentPubkeys,
          ...agentMentionPubkeys,
        ]);
        let sendChannelId = draft.capturedChannelId;
        if (preparedAgentPubkeys.length > 0 && onPrepareSendChannel) {
          sendChannelId = await onPrepareSendChannel(preparedAgentPubkeys);
          if (isSendCancelled()) return restoreComposerAfterFailure();
          if (!sendChannelId) {
            return restoreComposerAfterFailure();
          }
          if (!isMountedRef.current) {
            persistPreflightDraft();
            return;
          }
        }
        const agentReadiness = await ensureManagedAgentMentionsReady(
          managedMentionPubkeys.filter(
            (pubkey) => !readyAgentPubkeys.has(normalizePubkey(pubkey)),
          ),
          sendChannelId ?? "",
          onPrepareSendChannel ? preparedAgentPubkeys : [],
          [...managedAgentsByPubkey.values()],
        );
        if (isSendCancelled()) return restoreComposerAfterFailure();
        if (!isMountedRef.current) {
          persistPreflightDraft();
          return;
        }
        if (agentReadiness.errors.length > 0) {
          const message =
            agentReadiness.errors.length === 1
              ? `Could not start agent mention: ${agentReadiness.errors[0]}`
              : `Could not start agent mentions: ${agentReadiness.errors.join(
                  "; ",
                )}`;
          setNonMemberPromptError(message);
          toast.error(message);
          return restoreComposerAfterFailure();
        }
        if (preparedAgentPubkeys.length > 0 && sendChannelId) {
          try {
            await invokeTauri("sync_agents_to_active_huddle", {
              channelId: sendChannelId,
              agentPubkeys: preparedAgentPubkeys,
            });
            if (isSendCancelled()) return restoreComposerAfterFailure();
          } catch (error) {
            if (isSendCancelled()) return restoreComposerAfterFailure();
            const message = `Could not add mentioned agent to the Huddle: ${getErrorMessage(
              error,
              "Huddle enrollment failed.",
            )}`;
            setNonMemberPromptError(message);
            toast.error(message);
            return restoreComposerAfterFailure();
          }
        }
        const send = onSendRef.current;
        const finishSend = async (
          uploaded: ImetaMedia[],
          signal?: AbortSignal,
        ) => {
          const { content: finalContent, mediaTags } = buildOutgoingMessage(
            draft.trimmed,
            [...draft.savedImeta, ...uploaded],
            new Set([
              ...draft.savedSpoileredAttachmentUrls,
              ...draft.queuedAttachments.flatMap((attachment, index) =>
                attachment.spoilered && uploaded[index]
                  ? [uploaded[index].url]
                  : [],
              ),
            ]),
          );
          const finalOutgoingTags = await resolvePreviewTags(
            draft,
            mediaTags,
            outgoingTags,
          );
          if (!finalOutgoingTags || signal?.aborted || isSendCancelled())
            return;
          const revalidatedMentionPubkeys =
            await mentions.revalidateMentionPubkeys(mentionPubkeys);
          if (signal?.aborted || isSendCancelled()) return;
          const finalTagsWithAgentAddress = [
            ...finalOutgoingTags,
            ...buildAgentAddressMentionTags(
              draft.addressedAgentPubkeys,
              revalidatedMentionPubkeys,
            ),
          ];
          await send(
            finalContent,
            revalidatedMentionPubkeys,
            finalTagsWithAgentAddress,
            sendChannelId,
            draft.capturedThreadContext,
            draft.preparedLinkPreviews != null,
          );
          if (signal?.aborted || isSendCancelled()) return;
          const sentMentionPubkeys = new Set(
            revalidatedMentionPubkeys.map(normalizePubkey),
          );
          onInlineAgentMentionsSent?.({
            expectedRevision: draft.audienceRevision,
            pubkeys: draft.inlineAgentMentionPubkeys.filter((pubkey) =>
              sentMentionPubkeys.has(normalizePubkey(pubkey)),
            ),
          });
          if (draft.sentDraftKey) {
            drafts.markDraftSent(
              draft.sentDraftKey,
              draft.savedContent,
              draft.capturedChannelId ?? draft.sentDraftKey,
              draft.savedImeta,
              [...draft.savedSpoileredAttachmentUrls],
            );
          }
        };
        if (preparedUpload) {
          uploadStarted = preparedUpload.start({
            onComplete: async (uploaded, signal) => {
              try {
                await finishSend(uploaded, signal);
              } catch {
                restoreComposerAfterFailure();
              }
            },
            onError: (error) => {
              restoreComposerAfterFailure();
              toast.error(
                `Upload failed: ${getErrorMessage(error, "Unknown error")}`,
              );
            },
            onCancel: () => {
              restoreComposerAfterFailure();
            },
          });
          if (!uploadStarted) {
            return restoreComposerAfterFailure();
          }
        }
        if (!preparedUpload) {
          try {
            await finishSend([]);
          } catch {
            restoreComposerAfterFailure();
          }
        }
      } catch (error) {
        restoreComposerAfterFailure();
        throw error;
      } finally {
        if (draft.preparedLinkPreviews) {
          activePreparedLinkPreviews.delete(draft.preparedLinkPreviews);
        }
        draft.preparedLinkPreviews?.release();
        if (!uploadStarted) preparedUpload?.cancel();
        isCompleteSendPendingRef.current = false;
        if (isMountedRef.current) {
          setIsCompleteSendPending(false);
        }
      }
    },
    [
      clearComposer,
      contentRef,
      drafts,
      ensureManagedAgentMentionsReady,
      getManagedAgentsByPubkey,
      mentions.isAgentPubkey,
      mentions.revalidateMentionPubkeys,
      onAddressedAgentsSendStarted,
      onAddressedAgentsSendFailed,
      onInlineAgentMentionsSent,
      onPrepareSendChannel,
      onSendRef,
      richText.setContent,
      setContent,
      setPendingImeta,
      restoreQueuedAttachments,
      setSpoileredAttachmentUrls,
      hasUnsavedMedia,
      mentions.restoreDraftMentionRefs,
      activePreparedLinkPreviews,
    ],
  );
  const sendMessageWithMentionFlow = React.useCallback(
    async ({
      addressedAgentPubkeys = [],
      audienceRevision = 0,
      capturedChannelId,
      capturedThreadContext = null,
      pendingImeta,
      queuedAttachments = [],
      linkPreviewTags = [],
      preparedLinkPreviews = null,
      sentDraftKey,
      recoveryDraftKey,
      spoileredAttachmentUrls = new Set(),
      trimmed,
    }: SendMessageWithMentionFlowInput) => {
      if (isMentionSendPendingRef.current) {
        return;
      }
      isMentionSendPendingRef.current = true;
      setIsMentionSendPending(true);
      const isSendCancelled = () =>
        preparedLinkPreviews?.signal.aborted === true;
      let sendPromoted = false;
      if (preparedLinkPreviews) {
        activePreparedLinkPreviews.add(preparedLinkPreviews);
      }
      try {
        if (isSendCancelled()) return;
        const dmThreadAgentMentionErrorMessage = dmThreadAgentMentionError({
          trimmed,
          isThreadReply: capturedThreadContext != null,
          channelType,
          extractMentionPersonas: mentions.extractMentionPersonas,
          extractMentionPubkeys: (text) =>
            mergeMentionRecipients(
              mentions.extractMentionPubkeys(text),
              addressedAgentPubkeys,
            ),
          isAgentPubkey: mentions.isAgentPubkey,
          hasResolvedMembers: mentions.hasResolvedMembers,
          memberPubkeys: mentions.memberPubkeys,
        });
        if (dmThreadAgentMentionErrorMessage) {
          setNonMemberPromptError(dmThreadAgentMentionErrorMessage);
          toast.error(dmThreadAgentMentionErrorMessage);
          return;
        }
        let effectiveChannelId = capturedChannelId;
        if (!effectiveChannelId && onPrepareSendChannel) {
          effectiveChannelId = await onPrepareSendChannel();
          if (isSendCancelled()) return;
          if (!effectiveChannelId) {
            return;
          }
        }
        const personaMentionResult = await createMentionedPersonaAgents(
          trimmed,
          effectiveChannelId ?? "",
        );
        if (isSendCancelled()) return;
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
        const explicitMentionPubkeys = uniqueNormalizedPubkeys([
          ...mentions.extractMentionPubkeys(trimmed),
          ...createdPersonaAgentPubkeys,
        ]);
        const pubkeys = mergeMentionRecipients(
          explicitMentionPubkeys,
          addressedAgentPubkeys,
        );
        const outgoingTags = [
          ...buildCustomEmojiTags(trimmed, customEmoji),
          ...linkPreviewTags,
        ];
        const nonMemberPubkeys =
          channelType === null ||
          channelType === "dm" ||
          !mentions.hasResolvedMembers
            ? []
            : uniqueNormalizedPubkeys(pubkeys).filter(
                (pubkey) => !mentions.memberPubkeys.has(pubkey),
              );
        let promptNonMemberPubkeys = nonMemberPubkeys.filter(
          (pubkey) =>
            !mentions.isManagedAgentPubkey(pubkey) &&
            !createdPersonaAgentPubkeySet.has(normalizePubkey(pubkey)),
        );
        if (promptNonMemberPubkeys.length > 0) {
          try {
            const managedAgentsByPubkey = await getManagedAgentsByPubkey();
            if (isSendCancelled()) return;
            promptNonMemberPubkeys = promptNonMemberPubkeys.filter(
              (pubkey) => !managedAgentsByPubkey.has(normalizePubkey(pubkey)),
            );
          } catch {}
        }
        const savedMentionRefs = mentions.getDraftMentionRefs(trimmed);
        const pendingDraft: PendingNonMemberMentionSend = {
          addressedAgentPubkeys: uniqueNormalizedPubkeys(addressedAgentPubkeys),
          audienceRevision,
          inlineAgentMentionPubkeys: uniqueNormalizedPubkeys(
            savedMentionRefs
              .filter((ref) => ref.isAgent)
              .map((ref) => ref.pubkey),
          ),
          capturedChannelId: effectiveChannelId,
          capturedThreadContext,
          trimmed,
          mentionPubkeys: pubkeys,
          nonMemberPubkeys: promptNonMemberPubkeys,
          outgoingTags,
          preparedLinkPreviews,
          preparedManagedAgents: personaMentionResult.agents,
          readyAgentPubkeys:
            channelType === "dm" && onPrepareSendChannel
              ? []
              : createdPersonaAgentPubkeys,
          savedContent: trimmed,
          savedImeta: [...pendingImeta],
          queuedAttachments: [...queuedAttachments],
          savedSpoileredAttachmentUrls: new Set(spoileredAttachmentUrls),
          sentDraftKey,
          recoveryDraftKey,
          savedMentionRefs,
        };
        if (promptNonMemberPubkeys.length > 0) {
          setNonMemberPromptError(null);
          setPendingNonMemberSend(pendingDraft);
          return;
        }
        sendPromoted = true;
        await completeSend(pendingDraft, pubkeys);
      } finally {
        if (!sendPromoted) {
          if (preparedLinkPreviews) {
            activePreparedLinkPreviews.delete(preparedLinkPreviews);
          }
          preparedLinkPreviews?.release();
        }
        isMentionSendPendingRef.current = false;
        setIsMentionSendPending(false);
      }
    },
    [
      completeSend,
      channelType,
      createMentionedPersonaAgents,
      customEmoji,
      getManagedAgentsByPubkey,
      mentions.extractMentionPersonas,
      mentions.extractMentionPubkeys,
      mentions.hasResolvedMembers,
      mentions.isAgentPubkey,
      mentions.isManagedAgentPubkey,
      mentions.memberPubkeys,
      mentions.getDraftMentionRefs,
      onPrepareSendChannel,
      activePreparedLinkPreviews,
    ],
  );
  const pendingNonMemberNames = React.useMemo(() => {
    if (!pendingNonMemberSend) return [];
    return pendingNonMemberSend.nonMemberPubkeys.map(
      (pubkey) =>
        mentions.getMentionDisplayName(pubkey) ?? truncatePubkey(pubkey),
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
    if (!canInviteNonMembers) {
      setNonMemberPromptError(PRIVATE_CHANNEL_ADD_DENIED_MESSAGE);
      return;
    }
    setNonMemberPromptError(null);
    void (async () => {
      const mentionPubkeys = uniqueNormalizedPubkeys(
        await mentions.revalidateMentionPubkeys([
          ...pendingNonMemberSend.mentionPubkeys,
          ...pendingNonMemberSend.nonMemberPubkeys,
        ]),
      );
      const admittedMentionPubkeys = new Set(mentionPubkeys);
      const originalNonMemberPubkeys = new Set(
        pendingNonMemberSend.nonMemberPubkeys.map(normalizePubkey),
      );
      const nonMemberPubkeys = [...originalNonMemberPubkeys].filter(
        admittedMentionPubkeys.has.bind(admittedMentionPubkeys),
      );
      const outgoingTags = (pendingNonMemberSend.outgoingTags ?? []).filter(
        (tag) =>
          tag[0] !== MENTION_REFERENCE_TAG ||
          !originalNonMemberPubkeys.has(normalizePubkey(tag[1] ?? "")),
      );
      const managedAgentsByPubkey = await getManagedAgentsByPubkey();
      if (!isMountedRef.current) return;
      const peoplePubkeys: string[] = [];
      const relayAgentPubkeys: string[] = [];
      for (const pubkey of nonMemberPubkeys) {
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
    canInviteNonMembers,
    completeSend,
    getManagedAgentsByPubkey,
    mentions.isAgentPubkey,
    mentions.revalidateMentionPubkeys,
    pendingNonMemberSend,
  ]);
  const dismissNonMemberPrompt = React.useCallback(() => {
    setPendingNonMemberSend(null);
    setNonMemberPromptError(null);
  }, []);
  return {
    isPreparingMentionSend:
      isMentionSendPending ||
      isCompleteSendPending ||
      attachAgentMutation.isPending ||
      createPersonaAgentMutation.isPending ||
      startAgentMutation.isPending,
    nonMemberPromptProps: {
      canInvite: canInviteNonMembers,
      error: nonMemberPromptError,
      isInvitePending:
        isMentionSendPending ||
        isCompleteSendPending ||
        addMembersMutation.isPending ||
        attachAgentMutation.isPending ||
        createPersonaAgentMutation.isPending ||
        startAgentMutation.isPending,
      names: pendingNonMemberNames,
      onDismiss: dismissNonMemberPrompt,
      onDoNothing: handleSendWithoutInviting,
      onInvite: handleInviteNonMembers,
      open: pendingNonMemberSend !== null,
    },
    sendMessageWithMentionFlow,
  };
}
