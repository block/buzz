import * as React from "react";
import { toast } from "sonner";
import {
  useAttachManagedAgentToChannelMutation,
  useCreateChannelManagedAgentMutation,
  useProvisionChannelManagedAgentMutation,
  useStartManagedAgentMutation,
} from "@/features/agents/hooks";
import {
  useAddChannelMembersMutation,
  useCanAddChannelMembers,
} from "@/features/channels/hooks";
import { PRIVATE_CHANNEL_ADD_DENIED_MESSAGE } from "@/features/channels/lib/channelMemberAdmission";
import { dmThreadAgentMentionError } from "@/features/messages/lib/dmThreadAgentMentionError";
import { filterEffectiveExplicitAgentPubkeys } from "@/features/messages/lib/effectiveExplicitAgentPubkeys";
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
import type { ChannelType } from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { buildCustomEmojiTags } from "@/shared/lib/customEmojiTags";
import { useAgentMentionPreparation } from "./useAgentMentionPreparation";
import {
  getErrorMessage,
  MENTION_REFERENCE_TAG,
  mergeOutgoingTagsWithReferenceMentions,
  type PendingNonMemberMentionSend,
  type SendMessageWithMentionFlowInput,
  resolvePreviewTags,
  uniqueNormalizedPubkeys,
} from "./useMentionSendFlow.helpers";
import {
  decideRequestMarking,
  requestAgentPubkeysFor,
} from "@/features/messages/lib/requestMarking";
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
      requestAgentPubkeys?: string[],
    ) => Promise<void>
  >;
  richText: Pick<
    UseRichTextEditorResult,
    "clearContent" | "setContent" | "restorePlainTextAndFocusEnd"
  >;
  setContent: (content: string) => void;
  setIsEmojiPickerOpen: React.Dispatch<React.SetStateAction<boolean>>;
  setPendingImeta: (pendingImeta: ImetaMedia[]) => void;
  hasUnsavedMedia: () => boolean;
  clearQueuedAttachments: () => void;
  restoreQueuedAttachments: (attachments: QueuedMediaAttachment[]) => void;
  setSpoileredAttachmentUrls?: React.Dispatch<
    React.SetStateAction<Set<string>>
  >;
  onSuccessfulExplicitAgentAudience?: (audience: {
    channelId: string;
    expectedGeneration: number;
    expectedRevision: number | null;
    explicitAgentPubkeys: string[];
  }) => void;
  resolvePostSendContent?: (effectiveExplicitAgentPubkeys: string[]) => string;
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
  onSendRef,
  richText,
  setContent,
  setIsEmojiPickerOpen,
  setPendingImeta,
  hasUnsavedMedia,
  clearQueuedAttachments,
  restoreQueuedAttachments,
  setSpoileredAttachmentUrls,
  onSuccessfulExplicitAgentAudience,
  resolvePostSendContent,
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
  const startAgentMutation = useStartManagedAgentMutation();
  const {
    getManagedAgentsByPubkey,
    ensureManagedAgentMentionsReady,
    createMentionedPersonaAgents,
  } = useAgentMentionPreparation({
    channelType,
    onPrepareSendChannel,
    mentions,
    attachAgentMutation,
    startAgentMutation,
    createPersonaAgentMutation,
    provisionPersonaAgentMutation,
  });

  const clearComposer = React.useCallback(
    (postSendContent = "") => {
      setPendingNonMemberSend(null);
      setNonMemberPromptError(null);
      setContent(postSendContent);
      contentRef.current = postSendContent;
      if (postSendContent) {
        richText.restorePlainTextAndFocusEnd(postSendContent);
        mentions.cancelMentionAutocomplete();
      } else richText.clearContent();
      setPendingImeta([]);
      clearQueuedAttachments();
      setSpoileredAttachmentUrls?.(new Set());
      if (!postSendContent) mentions.clearMentions();
      channelLinks.clearChannels();
      emojiAutocomplete.clearEmojis();
      setIsEmojiPickerOpen(false);
    },
    [
      channelLinks.clearChannels,
      contentRef,
      emojiAutocomplete.clearEmojis,
      mentions.cancelMentionAutocomplete,
      mentions.clearMentions,
      richText.clearContent,
      richText.restorePlainTextAndFocusEnd,
      setContent,
      setIsEmojiPickerOpen,
      setPendingImeta,
      clearQueuedAttachments,
      setSpoileredAttachmentUrls,
    ],
  );

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
      let uploadStarted = false;
      try {
        const admittedMentionPubkeys = uniqueNormalizedPubkeys(
          await mentions.revalidateMentionPubkeys(mentionPubkeys),
        );
        if (isSendCancelled()) return;
        if (!isMountedRef.current) return persistPreflightDraft();
        const admittedMentionPubkeySet = new Set(admittedMentionPubkeys);
        const readyAgentPubkeys = new Set(
          uniqueNormalizedPubkeys(draft.readyAgentPubkeys ?? []).filter(
            (pubkey) => admittedMentionPubkeySet.has(pubkey),
          ),
        );
        const managedAgentsByPubkey = await getManagedAgentsByPubkey();
        if (isSendCancelled()) return;
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
          if (isSendCancelled()) return;
          if (!sendChannelId) {
            return;
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
        if (isSendCancelled()) return;
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
          return;
        }
        if (preparedAgentPubkeys.length > 0 && sendChannelId) {
          try {
            await invokeTauri("sync_agents_to_active_huddle", {
              channelId: sendChannelId,
              agentPubkeys: preparedAgentPubkeys,
            });
            if (isSendCancelled()) return;
          } catch (error) {
            if (isSendCancelled()) return;
            const message = `Could not add mentioned agent to the Huddle: ${getErrorMessage(
              error,
              "Huddle enrollment failed.",
            )}`;
            setNonMemberPromptError(message);
            toast.error(message);
            return;
          }
        }
        const effectiveExplicitAgentPubkeys =
          filterEffectiveExplicitAgentPubkeys(
            draft.explicitAgentPubkeys,
            mentionPubkeys,
          );
        const send = onSendRef.current;
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
        const restoreComposerAfterFailure = () => {
          if (isSendCancelled()) return;
          persistCanceledDraft();
          const canRestoreCurrentComposer =
            isMountedRef.current &&
            (draft.capturedChannelId === channelIdRef.current ||
              channelIdRef.current === null) &&
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
          const revalidatedExplicitAgentPubkeys =
            filterEffectiveExplicitAgentPubkeys(
              draft.explicitAgentPubkeys,
              revalidatedMentionPubkeys,
            );
          // NIP-AD: the agent this message is addressed to. Any agent in the
          // final mention list counts, however it got there (autocomplete,
          // explicit picker, or a raw pubkey/URI). It becomes the `agent`
          // tag, and a disposition only resolves this request if signed by
          // it. Non-agent mentions are deliberately excluded: they still get
          // a `p` tag, but a mentioned human must not be able to close an
          // agent's obligation.
          //
          // The decision itself lives in `decideRequestMarking` so the
          // composer banner shows the sender the same answer this uses. It
          // was derived only here before, which is how a message could
          // silently become an obligation — or silently stop being one.
          const mentionedAgents = revalidatedMentionPubkeys.filter(
            mentions.isAgentPubkey,
          );
          const requestAgentPubkeys = requestAgentPubkeysFor(
            decideRequestMarking(
              mentionedAgents,
              draft.requestTrackingOptedOut,
            ),
          );
          await send(
            finalContent,
            revalidatedMentionPubkeys,
            finalOutgoingTags,
            sendChannelId,
            draft.capturedThreadContext,
            draft.preparedLinkPreviews != null,
            requestAgentPubkeys,
          );
          if (signal?.aborted || isSendCancelled()) return;
          if (revalidatedExplicitAgentPubkeys.length > 0) {
            onSuccessfulExplicitAgentAudience?.({
              channelId: sendChannelId ?? draft.capturedChannelId ?? "",
              expectedGeneration: draft.audienceGeneration,
              expectedRevision: draft.audienceRevision,
              explicitAgentPubkeys: revalidatedExplicitAgentPubkeys,
            });
          }
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
            return;
          }
        }
        if (
          draft.capturedChannelId === channelIdRef.current ||
          channelIdRef.current === null
        ) {
          clearComposer(
            resolvePostSendContent?.(effectiveExplicitAgentPubkeys),
          );
        }

        if (!preparedUpload) {
          try {
            await finishSend([]);
          } catch {
            restoreComposerAfterFailure();
          }
        }
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
      onPrepareSendChannel,
      onSendRef,
      onSuccessfulExplicitAgentAudience,
      resolvePostSendContent,
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
      audienceGeneration = 0,
      audienceRevision = null,
      requestTrackingOptedOut = false,
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
          extractMentionPubkeys: mentions.extractMentionPubkeys,
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
        const explicitAgentPubkeys = explicitMentionPubkeys.filter(
          (pubkey) =>
            mentions.isAgentPubkey(pubkey) ||
            createdPersonaAgentPubkeySet.has(pubkey),
        );
        const pubkeys = explicitMentionPubkeys;
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

        const pendingDraft: PendingNonMemberMentionSend = {
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
          savedMentionRefs: mentions.getDraftMentionRefs(trimmed),
          audienceGeneration,
          audienceRevision,
          requestTrackingOptedOut,
          explicitAgentPubkeys,
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
