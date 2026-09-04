import * as React from "react";
import { claimDraftSend } from "@/features/messages/lib/useDrafts";
import { toast } from "sonner";
import {
  type CreateChannelManagedAgentInput,
  useAttachManagedAgentToChannelMutation,
  useAvailableAcpRuntimes,
  useCreateChannelManagedAgentMutation,
  useManagedAgentsQuery,
  usePersonasQuery,
  useProvisionChannelManagedAgentMutation,
} from "@/features/agents/hooks";
import { resolvePersonaRuntime } from "@/features/agents/lib/resolvePersonaRuntime";
import { useAddChannelMembersMutation } from "@/features/channels/hooks";
import { useCanAddChannelMembers } from "@/features/channels/useCanAddChannelMembers";
import { useNonMemberInvite } from "./useNonMemberInvite";
import { dmThreadAgentMentionError } from "@/features/messages/lib/dmThreadAgentMentionError";
import {
  prepareBackgroundMediaUpload,
  saveQueuedAttachmentsForDraft,
} from "@/features/messages/lib/backgroundMediaUploadStore";
import {
  buildOutgoingMessage,
  type ImetaMedia,
} from "@/features/messages/lib/imetaMediaMarkdown";
import { useActivePreparedLinkPreviews } from "./useActivePreparedLinkPreviews";
import { useDetachedAgentStart } from "./useDetachedAgentStart";
import { useEnsureAgentMentionsReady } from "./useEnsureAgentMentionsReady";
import { invokeTauri } from "@/shared/api/tauri";
import type { AcpRuntime, ManagedAgent } from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { buildCustomEmojiTags } from "@/shared/lib/customEmojiTags";
import {
  dedupeQueuedAgentWakes,
  enqueueAgentWake,
  formatMessageSendError,
  getErrorMessage,
  mentionRevalidationOptions,
  withoutInvitingRecipients,
  mergeMentionRecipients,
  type PendingNonMemberMentionSend,
  type QueuedAgentWake,
  type SendMessageWithMentionFlowInput,
  resolvePreviewTags,
  uniqueNormalizedPubkeys,
} from "./useMentionSendFlow.helpers";
import { buildAgentAddressMentionTags } from "@/features/messages/lib/agentAddressMention.mjs";
import { AgentMentionAuthorizationError } from "@/features/messages/lib/agentMentionRevalidation";
import type { UseMentionSendFlowOptions } from "./useMentionSendFlow.types";

export function useMentionSendFlow({
  channelId,
  effectiveDraftKey,
  getComposerRevision,
  runComposerUpdate,
  channelLinks,
  channelType,
  contentRef,
  customEmoji,
  drafts,
  emojiAutocomplete,
  mentions,
  onPrepareSendChannel,
  onAddressedAgentsComposerCleared,
  onAddressedAgentsSendFailed,
  onAddressedAgentsSendSucceeded,
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
  // Persistence identity is independent of the host component and destination
  // channel. A -> B -> A must not revive A's previous invitation or recovery.
  const sourceOwner = React.useMemo(
    () => ({ channelId, draftKey: effectiveDraftKey, getComposerRevision }),
    [channelId, effectiveDraftKey, getComposerRevision],
  );
  const sourceOwnerRef = React.useRef(sourceOwner);
  sourceOwnerRef.current = sourceOwner;
  const [completeSendOwner, setCompleteSendOwner] = React.useState<
    typeof sourceOwner | null
  >(null);
  const isCompleteSendPending = completeSendOwner === sourceOwner;
  const isMentionSendPendingRef = React.useRef(false);
  const completeSendRef = React.useRef<PendingNonMemberMentionSend | null>(
    null,
  );
  const isMountedRef = React.useRef(false);
  const activePreparedLinkPreviews = useActivePreparedLinkPreviews();

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
  const personasQuery = usePersonasQuery();
  // Detached (publish-first) agent wake, bound to the community and identity
  // active at this render so a start that outlives a community switch fails
  // closed instead of spawning against the new tenant. The send path never
  // calls it while preparing a message: wakes are queued during preparation
  // and flushed through this callback only after the relay accepts the
  // publish, so a start failure can never toast "your message was sent"
  // before the publish outcome is known.
  const startAgentDetached = useDetachedAgentStart();
  const getManagedAgentsByPubkey = React.useCallback(async () => {
    const agents =
      managedAgentsQuery.data ??
      (await managedAgentsQuery.refetch()).data ??
      [];
    return new Map(
      agents.map((agent) => [normalizePubkey(agent.pubkey), agent]),
    );
  }, [managedAgentsQuery.data, managedAgentsQuery.refetch]);
  const getPersonas = React.useCallback(async () => {
    return personasQuery.data ?? (await personasQuery.refetch()).data ?? [];
  }, [personasQuery.data, personasQuery.refetch]);
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
  const ensureManagedAgentMentionsReady = useEnsureAgentMentionsReady({
    attachAgentToChannel: attachAgentMutation.mutateAsync,
    getManagedAgentsByPubkey,
    getPersonas,
    memberPubkeys: mentions.memberPubkeys,
  });
  const createMentionedPersonaAgents = React.useCallback(
    async (
      personaMentions: ReturnType<typeof mentions.extractMentionPersonas>,
      capturedChannelId: string,
    ) => {
      if (!capturedChannelId || personaMentions.length === 0) {
        return {
          errors: [] as string[],
          agents: [] as ManagedAgent[],
          pubkeys: [] as string[],
          agentsToWake: [] as QueuedAgentWake[],
          mentionRefs: [] as PendingNonMemberMentionSend["savedMentionRefs"],
        };
      }
      const runtimes = await getAvailableRuntimes();
      const defaultRuntime = runtimes[0] ?? null;
      const errors: string[] = [];
      const agents: ManagedAgent[] = [];
      const pubkeys: string[] = [];
      // Queued, not fired: the wakes ride the pending draft and flush only
      // after the publish succeeds, so a persona created for a send the
      // non-member prompt later cancels never wakes at all.
      const agentsToWake: QueuedAgentWake[] = [];
      const mentionRefs: PendingNonMemberMentionSend["savedMentionRefs"] = [];
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
            detachedStart: (agentToWake) =>
              enqueueAgentWake(agentsToWake, agentToWake),
          };
          const result = shouldProvisionForDm
            ? await provisionPersonaAgentMutation.mutateAsync(input)
            : await createPersonaAgentMutation.mutateAsync(input);
          const pubkey = normalizePubkey(result.agent.pubkey);
          agents.push(result.agent);
          pubkeys.push(pubkey);
          mentionRefs.push({ displayName, pubkey, isAgent: true });
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
        agentsToWake,
        mentionRefs,
      };
    },
    [
      createPersonaAgentMutation,
      channelType,
      getAvailableRuntimes,
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
  React.useLayoutEffect(() => {
    setPendingNonMemberSend((draft) =>
      draft?.sourceOwner === sourceOwner ? draft : null,
    );
    setNonMemberPromptError(null);
  }, [sourceOwner]);
  const completeSend = React.useCallback(
    async (
      draft: PendingNonMemberMentionSend,
      mentionPubkeys: string[],
      outgoingTags = draft.outgoingTags,
    ) => {
      const pending = completeSendRef.current;
      if (
        pending?.sourceOwner === draft.sourceOwner &&
        !pending.invitationSignal?.aborted
      )
        return;
      const ownsComposer = () => sourceOwnerRef.current === draft.sourceOwner;
      const sendSignal = draft.preparedLinkPreviews?.signal;
      const isSendCancelled = () =>
        sendSignal?.aborted === true ||
        draft.invitationSignal?.aborted === true;
      if (isSendCancelled()) return draft.preparedLinkPreviews?.release();
      completeSendRef.current = draft;
      setCompleteSendOwner(draft.sourceOwner);
      const preparedUpload =
        draft.queuedAttachments.length > 0
          ? prepareBackgroundMediaUpload(draft.queuedAttachments)
          : null;
      const persistRecoverableDraft = () => {
        // Invitation cancellation still owes the captured draft recovery. Link
        // preview cancellation retains its existing independent recovery owner.
        if (
          sendSignal?.aborted ||
          !draft.recoveryDraftKey ||
          draft.sourceOwner.getComposerRevision() !== draft.composerRevision
        )
          return false;
        const existing = drafts.loadDraft(draft.recoveryDraftKey);
        if (
          existing &&
          (existing.content !== draft.savedContent ||
            existing.channelId !==
              (draft.capturedChannelId ?? draft.recoveryDraftKey) ||
            JSON.stringify(existing.pendingImeta) !==
              JSON.stringify(draft.savedImeta) ||
            JSON.stringify(existing.spoileredAttachmentUrls) !==
              JSON.stringify([...draft.savedSpoileredAttachmentUrls]) ||
            JSON.stringify(existing.mentionRefs ?? []) !==
              JSON.stringify(draft.savedMentionRefs))
        ) {
          return false;
        }
        drafts.persistDraft(
          draft.recoveryDraftKey,
          draft.savedContent,
          draft.capturedChannelId ?? draft.recoveryDraftKey,
          draft.savedImeta,
          [...draft.savedSpoileredAttachmentUrls],
          draft.savedMentionRefs,
        );
        return true;
      };
      const persistPreflightDraft = () => {
        if (
          !draft.recoveryDraftKey ||
          isSendCancelled() ||
          !persistRecoverableDraft()
        )
          return;
        saveQueuedAttachmentsForDraft(
          draft.recoveryDraftKey,
          draft.queuedAttachments,
        );
      };
      let composerCleared = false;
      let optimisticComposerContent = "";
      let clearedRevision = -1;
      const restoreComposerAfterFailure = () => {
        if (!composerCleared) return;
        composerCleared = false;
        // An authored edit (even edit -> clear) ends optimistic recovery's
        // authority over this key, including its persisted record and files.
        if (draft.sourceOwner.getComposerRevision() !== clearedRevision) return;
        const persisted = persistRecoverableDraft();
        const canAnimateCurrentComposer =
          isMountedRef.current && ownsComposer();
        if (
          canAnimateCurrentComposer &&
          draft.addressedAgentPubkeys.length > 0
        ) {
          onAddressedAgentsSendFailed?.(draft.addressedAgentPubkeys);
        }
        const canRestoreCurrentComposer =
          canAnimateCurrentComposer &&
          getComposerRevision() === clearedRevision &&
          contentRef.current.trim() === optimisticComposerContent.trim() &&
          !hasUnsavedMedia();
        if (!canRestoreCurrentComposer && persisted && draft.recoveryDraftKey) {
          saveQueuedAttachmentsForDraft(
            draft.recoveryDraftKey,
            draft.queuedAttachments,
          );
        }
        if (!canRestoreCurrentComposer) {
          return;
        }
        runComposerUpdate(() => {
          setContent(draft.savedContent);
          contentRef.current = draft.savedContent;
          richText.setContent(draft.savedContent);
          setPendingImeta(draft.savedImeta);
          restoreQueuedAttachments(draft.queuedAttachments);
          mentions.restoreDraftMentionRefs(draft.savedMentionRefs);
          setSpoileredAttachmentUrls?.(
            new Set(draft.savedSpoileredAttachmentUrls),
          );
        });
      };
      if (ownsComposer() && getComposerRevision() === draft.composerRevision) {
        runComposerUpdate(() => {
          clearComposer();
          if (draft.addressedAgentPubkeys.length > 0) {
            optimisticComposerContent =
              onAddressedAgentsComposerCleared?.(draft.addressedAgentPubkeys) ??
              "";
            contentRef.current = optimisticComposerContent;
          }
        });
        composerCleared = true;
        clearedRevision = getComposerRevision();
      }
      // Recover on invalidation, not when an arbitrary external await settles.
      // In particular a later visit to A must load recovery before it can edit
      // or clear A again. The async continuation only observes cancellation.
      const cancelSend = () => {
        restoreComposerAfterFailure();
        if (completeSendRef.current === draft) {
          completeSendRef.current = null;
          if (isMountedRef.current) setCompleteSendOwner(null);
        }
      };
      draft.invitationSignal?.addEventListener("abort", cancelSend, {
        once: true,
      });
      let uploadStarted = false;
      try {
        const admittedMentionPubkeys = uniqueNormalizedPubkeys(
          await mentions.revalidateMentionPubkeys(
            mentionPubkeys,
            draft.capturedChannelId,
            mentionRevalidationOptions(draft, "prepare"),
          ),
        );
        if (isSendCancelled()) return restoreComposerAfterFailure();
        if (!isMountedRef.current) return persistPreflightDraft();
        const admittedMentionPubkeySet = new Set(admittedMentionPubkeys);
        const readyAgentPubkeys = new Set(
          uniqueNormalizedPubkeys(draft.readyAgentPubkeys ?? []).filter(
            (pubkey) => admittedMentionPubkeySet.has(pubkey),
          ),
        );
        const managedAgentsByPubkey = await getManagedAgentsByPubkey().catch(
          () => new Map<string, ManagedAgent>(),
        );
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
          isSendCancelled,
        );
        // Every wake this send queued: persona creates carried on the draft
        // (enqueued before the non-member prompt could defer us here), then
        // the readiness pass's. Flushed only after the relay accepts the
        // publish — every abort path between here and there just drops them,
        // so no wake (or "your message was sent" failure toast) can exist for
        // a message that never landed. First entry wins the dedupe because it
        // carries the earliest replay floor, and the floor is a lower bound.
        const agentsToWake = dedupeQueuedAgentWakes([
          ...(draft.queuedAgentWakes ?? []),
          ...agentReadiness.agentsToWake,
        ]);
        if (isSendCancelled()) return restoreComposerAfterFailure();
        if (!isMountedRef.current) {
          persistPreflightDraft();
          return;
        }
        if (agentReadiness.errors.length > 0) {
          const message =
            agentReadiness.errors.length === 1
              ? `Could not prepare agent mention: ${agentReadiness.errors[0]}`
              : `Could not prepare agent mentions: ${agentReadiness.errors.join(
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
            return restoreComposerAfterFailure();
          // The pass immediately before signing/publish is always fresh:
          // mention authorization is re-validated here unconditionally,
          // whatever did or did not separate it from the admission pass
          // above (#5681).
          const revalidatedMentionPubkeys =
            await mentions.revalidateMentionPubkeys(
              mentionPubkeys,
              sendChannelId,
              mentionRevalidationOptions(
                draft,
                "publish",
                preparedAgentPubkeys,
              ),
            );
          if (signal?.aborted || isSendCancelled())
            return restoreComposerAfterFailure();
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
          // The relay accepted the publish: flush the queued wakes now,
          // before the post-send cancellation check — a cancellation racing
          // a successful publish must not drop the wake for a message that
          // did land. Fire-and-forget: the send awaits nothing here, and
          // each wake carries its enqueue-time replay floor so the spawned
          // harness replays back past this message however late the flush.
          for (const wake of agentsToWake) {
            startAgentDetached(wake.agent, wake.replayFloorUnix);
          }
          if (signal?.aborted || isSendCancelled()) return;
          const sentMentionPubkeys = new Set(
            revalidatedMentionPubkeys.map(normalizePubkey),
          );
          const newlyPinnedPubkeys = draft.inlineAgentMentionPubkeys.filter(
            (pubkey) => sentMentionPubkeys.has(normalizePubkey(pubkey)),
          );
          if (
            ownsComposer() &&
            getComposerRevision() === draft.composerRevision
          ) {
            onAddressedAgentsSendSucceeded?.(
              [
                ...new Set([
                  ...draft.addressedAgentPubkeys,
                  ...newlyPinnedPubkeys,
                ]),
              ],
              newlyPinnedPubkeys,
            );
          }
          if (
            draft.sentDraftKey &&
            draft.sourceOwner.getComposerRevision() ===
              draft.composerRevision &&
            JSON.stringify(
              drafts.loadDraft(draft.sentDraftKey)?.mentionRefs ?? [],
            ) === JSON.stringify(draft.savedMentionRefs)
          ) {
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
          let settleUpload!: () => void;
          const uploadSettled = new Promise<void>((resolve) => {
            settleUpload = resolve;
          });
          uploadStarted = preparedUpload.start({
            onComplete: async (uploaded, signal) => {
              try {
                await finishSend(uploaded, signal);
              } catch (error) {
                restoreComposerAfterFailure();
                toast.error(
                  error instanceof AgentMentionAuthorizationError
                    ? error.message
                    : formatMessageSendError(error),
                );
              } finally {
                settleUpload();
              }
            },
            onError: (error) => {
              restoreComposerAfterFailure();
              toast.error(
                `Upload failed: ${getErrorMessage(error, "Unknown error")}`,
              );
              settleUpload();
            },
            onCancel: () => {
              restoreComposerAfterFailure();
              settleUpload();
            },
          });
          if (!uploadStarted) {
            settleUpload();
            return restoreComposerAfterFailure();
          }
          await uploadSettled;
        }
        if (!preparedUpload) {
          try {
            await finishSend([]);
          } catch (error) {
            restoreComposerAfterFailure();
            toast.error(
              error instanceof AgentMentionAuthorizationError
                ? error.message
                : formatMessageSendError(error),
            );
          }
        }
      } catch (error) {
        restoreComposerAfterFailure();
        toast.error(
          getErrorMessage(error, "Could not send message. Please retry."),
        );
      } finally {
        if (draft.preparedLinkPreviews) {
          activePreparedLinkPreviews.delete(draft.preparedLinkPreviews);
        }
        draft.preparedLinkPreviews?.release();
        if (!uploadStarted) preparedUpload?.cancel();
        draft.invitationSignal?.removeEventListener("abort", cancelSend);
        if (completeSendRef.current === draft) {
          completeSendRef.current = null;
          if (isMountedRef.current) setCompleteSendOwner(null);
        }
      }
    },
    [
      clearComposer,
      getComposerRevision,
      runComposerUpdate,
      contentRef,
      drafts,
      ensureManagedAgentMentionsReady,
      getManagedAgentsByPubkey,
      mentions.isAgentPubkey,
      mentions.revalidateMentionPubkeys,
      onAddressedAgentsComposerCleared,
      onAddressedAgentsSendFailed,
      onAddressedAgentsSendSucceeded,
      onPrepareSendChannel,
      onSendRef,
      richText.setContent,
      setContent,
      startAgentDetached,
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
      // Bind settlement to this authored visit before reading its recipients.
      claimDraftSend(effectiveDraftKey);
      const composerRevision = getComposerRevision();
      const isSendCancelled = () =>
        preparedLinkPreviews?.signal.aborted === true;
      let sendPromoted = false;
      if (preparedLinkPreviews) {
        activePreparedLinkPreviews.add(preparedLinkPreviews);
      }
      try {
        if (isSendCancelled()) return;
        // Every extraction below reads the mention map, and a pasted identity
        // can still be verifying — the relay round trip for a non-member is
        // exactly the case this feature exists for. Sending first would
        // publish a readable `@Label` with no `p` tag. Bounded inside, so a
        // lookup that never answers delays the send rather than blocking it.
        await mentions.settlePendingMentionBindings();
        // Settlement may outlive an edit or A → B → A navigation. In that
        // case the live mention maps no longer belong to this send.
        if (
          isSendCancelled() ||
          !isMountedRef.current ||
          sourceOwnerRef.current !== sourceOwner ||
          getComposerRevision() !== composerRevision
        )
          return;
        const savedMentionRefs = mentions.getDraftMentionRefs(trimmed).slice();
        const selectedMentionPubkeys = mentions.extractMentionPubkeys(trimmed);
        const selectedPersonas = mentions.extractMentionPersonas(trimmed);
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
          selectedPersonas,
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
        savedMentionRefs.push(...personaMentionResult.mentionRefs);
        // Preparation resolves the captured persona, never the editor's new
        // selection. Only this unchanged visit may receive its resolved binding.
        if (
          isMountedRef.current &&
          sourceOwnerRef.current === sourceOwner &&
          getComposerRevision() === composerRevision
        ) {
          for (const ref of personaMentionResult.mentionRefs)
            mentions.registerMentionPubkey(ref.displayName, ref.pubkey, ref);
        }
        const explicitMentionPubkeys = uniqueNormalizedPubkeys([
          ...selectedMentionPubkeys,
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
        const pendingDraft: PendingNonMemberMentionSend = {
          sourceOwner,
          composerRevision,
          addressedAgentPubkeys: uniqueNormalizedPubkeys(addressedAgentPubkeys),
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
          queuedAgentWakes: personaMentionResult.agentsToWake,
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
          if (sourceOwnerRef.current !== sourceOwner) return;
          setNonMemberPromptError(null);
          setPendingNonMemberSend(pendingDraft);
          return;
        }
        sendPromoted = true;
        await completeSend(pendingDraft, pubkeys);
      } catch (error) {
        toast.error(
          getErrorMessage(error, "Could not prepare mentions. Please retry."),
        );
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
      effectiveDraftKey,
      sourceOwner,
      getComposerRevision,
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
      mentions.settlePendingMentionBindings,
      mentions.registerMentionPubkey,
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
  const invitation = useNonMemberInvite({
    sourceOwner,
    draft: pendingNonMemberSend,
    canInvite: canInviteNonMembers,
    revalidate: mentions.revalidateMentionPubkeys,
    getManagedAgentsByPubkey,
    isAgentPubkey: mentions.isAgentPubkey,
    addMembers: addMembersMutation.mutateAsync,
    completeSend,
    setError: setNonMemberPromptError,
  });
  const handleSendWithoutInviting = React.useCallback(() => {
    if (!pendingNonMemberSend) return;
    invitation.cancel();
    const { mentionPubkeys, outgoingTags } =
      withoutInvitingRecipients(pendingNonMemberSend);
    void completeSend(pendingNonMemberSend, mentionPubkeys, outgoingTags);
  }, [completeSend, pendingNonMemberSend, invitation.cancel]);
  const dismissNonMemberPrompt = React.useCallback(() => {
    invitation.cancel();
    setPendingNonMemberSend(null);
    setNonMemberPromptError(null);
  }, [invitation.cancel]);
  return {
    // Agent starts are detached (publish-first), so useDetachedAgentStart's
    // in-flight state deliberately does not gate the composer — a background
    // start must not block the next send.
    isPreparingMentionSend:
      invitation.isPending ||
      isMentionSendPending ||
      isCompleteSendPending ||
      attachAgentMutation.isPending ||
      createPersonaAgentMutation.isPending,
    nonMemberPromptProps: {
      canInvite: canInviteNonMembers,
      error: nonMemberPromptError,
      isInvitePending:
        invitation.isPending ||
        isMentionSendPending ||
        isCompleteSendPending ||
        attachAgentMutation.isPending ||
        createPersonaAgentMutation.isPending,
      names: pendingNonMemberNames,
      onDismiss: dismissNonMemberPrompt,
      onDoNothing: handleSendWithoutInviting,
      onInvite: invitation.invite,
      open: pendingNonMemberSend !== null,
    },
    sendMessageWithMentionFlow,
  };
}
