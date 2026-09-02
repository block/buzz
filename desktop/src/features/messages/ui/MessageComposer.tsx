import * as React from "react";
import { EditorContent } from "@tiptap/react";
import {
  useChannelLinks,
  type ChannelSuggestion,
} from "@/features/messages/lib/useChannelLinks";
import { useComposerAutofocus } from "@/features/messages/lib/useComposerAutofocus";
import { useDrafts } from "@/features/messages/lib/useDrafts";
import { resolveSentDraftKey } from "@/features/messages/ui/draftSubmitKey";
import {
  useEmojiAutocomplete,
  type EmojiSuggestion,
} from "@/features/messages/lib/useEmojiAutocomplete";
import { useCustomEmoji } from "@/features/custom-emoji/hooks";
import {
  findSpoileredImetaMediaUrls,
  type ImetaMedia,
  restoreImetaMediaDisplayLabels,
  stripImetaMediaLines,
} from "@/features/messages/lib/imetaMediaMarkdown";
import { useMediaUpload } from "@/features/messages/lib/useMediaUpload";
import {
  cancelBackgroundMediaUploads,
  saveQueuedAttachmentsForDraft,
  takeQueuedAttachmentsForDraft,
  useBackgroundMediaUpload,
} from "@/features/messages/lib/backgroundMediaUploadStore";
import { useComposerFocusOwnership } from "@/features/messages/lib/useComposerFocusOwnership";
import { isMentionCodeContext } from "@/features/messages/lib/mentionCodeContext";
import { useMentions } from "@/features/messages/lib/useMentions";
import { getPersistentAgentAudienceScope } from "@/features/messages/lib/persistentAgentAudience";
import { setKeepMentionedAgentsPinned } from "@/features/messages/lib/autoPinMentionedAgentsPreference";
import { useIdentityQuery } from "@/shared/api/hooks";
import { CUSTOM_EMOJI_NODE_NAME } from "@/features/messages/lib/customEmojiNode";
import {
  type AutocompleteEdit,
  type LinkSelectionInfo,
  useRichTextEditor,
} from "@/features/messages/lib/useRichTextEditor";
import { useLinkEditor } from "@/features/messages/lib/useLinkEditor";
import { useComposerSpoilerParticles } from "@/features/messages/lib/useComposerSpoilerParticles";
import { useTypingBroadcast } from "@/features/messages/useTypingBroadcast";
import { cn } from "@/shared/lib/cn";
import { ComposerReplyEditBanner } from "./ComposerReplyEditBanner";
import { ComposerAttachments, DropZoneOverlay } from "./ComposerAttachments";
import { focusMentionOptionsTrigger } from "./MentionAutocomplete";
import { MessageComposerAutocompletes } from "./MessageComposerAutocompletes";
import { ComposerDockToolbar } from "./ComposerDockToolbar";
import { ComposerUploadProgressPill } from "./ComposerUploadProgressPill";
import { NonMemberMentionDialog } from "./NonMemberMentionDialog";
import { useComposerVoiceNote } from "./useComposerVoiceNote";
import { useMentionSendFlow } from "./useMentionSendFlow";
import { useAgentAddressLockPicker } from "./useAgentAddressLockPicker";
import { useAddressMentionPulse } from "./useAddressMentionPulse";
import { useAlwaysAddressShortcut } from "./useAlwaysAddressShortcut";
import { useComposerMentionPicker } from "./useComposerMentionPicker";
import { useAutoPinMentionedAgents } from "./useAutoPinMentionedAgents";
import { useComposerAttachmentSpoilers } from "./useComposerAttachmentSpoilers";
import { useComposerContentState } from "./useComposerContentState";
import { useComposerPasteHandler } from "./useComposerPasteHandler";
import { useDraftPersistLifecycle } from "./useDraftPersistSnapshot";
import { useImplicitAgentMentionProvenance } from "./useImplicitAgentMentionProvenance";
import { useThreadAgentAudience } from "./useThreadAgentAudience";
import { submitMessageEdit } from "./submitMessageEdit";
import { prepareBackgroundLinkPreviews } from "@/features/messages/lib/linkPreviewPreparationStore";
import { useComposerLinkPreviews } from "./useComposerLinkPreviews";
import { useAddressedAgentMentionRestore } from "./useAddressedAgentMentionRestore";
import { scheduleSettleGatedAutoSubmit } from "./messageComposerAutoSubmit";
import type { MessageComposerProps } from "./MessageComposer.types";
function MessageComposerImpl({
  audienceContext = null,
  channelId = null,
  channelName,
  channelType = null,
  containerClassName,
  layoutMode = "standalone",
  disabled = false,
  draftKey,
  autoSubmitDraftKey = null,
  onAutoSubmitComplete,
  editTarget = null,
  isSending = false,
  onAttachmentAcceptanceChange,
  onDeferredEditPendingChange,
  onCancelEdit,
  onCancelReply,
  onCaptureSendContext,
  onEditLastOwnMessage,
  onEditSave,
  onPrepareSendChannel,
  onPreparingMentionSendChange,
  onSend,
  placeholder,
  profiles,
  recentMentionPubkeys,
  replyTarget = null,
  mediaController,
  showBackgroundUploadProgress = true,
  showTopBorder = false,
  toolbarExtraActions,
  typingParentEventId = null,
  typingRootEventId = null,
}: MessageComposerProps) {
  const {
    contentRef,
    isContentEmpty,
    setComposerContent,
    setComposerContentFromText,
    syncComposerContentFromEditor,
    syncContentRefFromEditorRef,
  } = useComposerContentState();
  const [previewContent, setPreviewContent] = React.useState("");
  const {
    previewList: composerLinkPreviews,
    getLiveCandidates: getLiveLinkPreviewCandidates,
    getReadyTags: getReadyLinkPreviewTags,
  } = useComposerLinkPreviews(previewContent, editTarget == null);
  const [isEmojiPickerOpen, setIsEmojiPickerOpen] = React.useState(false);
  const [isFormattingOpen, setIsFormattingOpen] = React.useState(false);
  const handleFormattingToggle = React.useCallback((pressed: boolean) => {
    if (pressed) setIsEmojiPickerOpen(false);
    setIsFormattingOpen(pressed);
  }, []);
  const drafts = useDrafts();
  const identityQuery = useIdentityQuery();
  const effectiveDraftKey = draftKey ?? channelId;
  const ownerPubkey = identityQuery.data?.pubkey ?? null;
  const audienceScope =
    audienceContext && channelId && ownerPubkey
      ? getPersistentAgentAudienceScope({
          ownerPubkey,
          channelId,
          composerKey: effectiveDraftKey,
        })
      : null;
  const effectiveDraftKeyRef = React.useRef(effectiveDraftKey);
  effectiveDraftKeyRef.current = effectiveDraftKey;
  const implicitAgentMentionProvenance =
    useImplicitAgentMentionProvenance(effectiveDraftKey);
  const preEditSnapshotRef = React.useRef<{
    content: string;
    pendingImeta: ImetaMedia[];
    queuedAttachments: ReturnType<typeof useMediaUpload>["queuedAttachments"];
    spoileredAttachmentUrls: Set<string>;
  } | null>(null);
  const mentions = useMentions(channelId, undefined, profiles, {
    channelType,
    recentMentionPubkeys,
  });
  const channelLinks = useChannelLinks();
  const customEmoji = useCustomEmoji();
  const emojiAutocomplete = useEmojiAutocomplete(customEmoji);
  const notifyTyping = useTypingBroadcast(
    channelId,
    typingParentEventId,
    typingRootEventId,
  );
  const internalMedia = useMediaUpload({ deferUploadsUntilSend: true });
  const media = mediaController ?? internalMedia;
  const voiceNote = useComposerVoiceNote({
    draftKey: effectiveDraftKey,
    editTargetId: editTarget?.id ?? null,
    media,
    setFormattingOpen: setIsFormattingOpen,
    setEmojiPickerOpen: setIsEmojiPickerOpen,
  });
  React.useEffect(() => {
    onAttachmentAcceptanceChange?.(voiceNote.acceptsAttachment);
  }, [onAttachmentAcceptanceChange, voiceNote.acceptsAttachment]);
  const {
    handleAttachmentEditSave,
    handleAttachmentRevert,
    handleRemoveAttachment,
    handleToggleAttachmentSpoiler,
    setSpoileredAttachmentUrls,
    spoileredAttachmentUrls,
    spoileredAttachmentUrlsRef,
  } = useComposerAttachmentSpoilers({
    removeAttachment: media.removeAttachment,
    revertAttachment: media.revertAttachment,
    uploadEditedAttachment: media.uploadEditedAttachment,
  });
  const [isDeferredEditPending, setDeferredEditPending] = React.useState(false);
  const composerDisabled = disabled || isDeferredEditPending;
  const isEditSubmissionLocked =
    isSending || media.isUploading || isDeferredEditPending;
  const canRestoreEditDraftRef = React.useRef(false);
  canRestoreEditDraftRef.current =
    contentRef.current.trim().length === 0 &&
    media.pendingImetaRef.current.length === 0 &&
    media.queuedAttachmentsRef.current.length === 0;
  const ownsDropZone = mediaController === undefined;
  const backgroundUpload = useBackgroundMediaUpload();
  const { trackAuthoredContent } = useDraftPersistLifecycle({
    effectiveDraftKey,
    channelId,
    loadDraft: drafts.loadDraft,
    persistDraft: drafts.persistDraft,
    getMentionRefs: mentions.getDraftMentionRefs,
    restoreMentionRefs: mentions.restoreDraftMentionRefs,
    livePendingImeta: media.pendingImeta,
    setPendingImeta: media.setPendingImeta,
    getQueuedAttachments: () => media.queuedAttachmentsRef.current,
    saveQueuedAttachmentsForDraft,
    clearQueuedAttachments: media.clearQueuedAttachments,
    restoreQueuedAttachments: media.restoreQueuedAttachments,
    takeQueuedAttachmentsForDraft,
    setContent: (content) => {
      setComposerContent(content);
      richText.setContent(content);
    },
    clearContent: () => {
      setComposerContent("");
      richText.clearContent();
    },
    setSpoileredAttachmentUrls,
    spoileredAttachmentUrlsRef,
    syncComposerContentFromEditor,
    getImplicitAgentMentionPrefix: implicitAgentMentionProvenance.getPrefix,
  });
  // biome-ignore lint/correctness/useExhaustiveDependencies: effectiveDraftKey is the sole trigger
  React.useEffect(() => {
    media.setUploadState({ status: "idle" });
    setIsEmojiPickerOpen(false);
    channelLinks.clearChannels();
    emojiAutocomplete.clearEmojis();
  }, [effectiveDraftKey]);
  const disabledRef = React.useRef(disabled);
  const isSendingRef = React.useRef(isSending);
  const isUploadingRef = React.useRef(media.isUploading);
  const isSubmitLockedRef = React.useRef(false);
  const [isSubmitLocked, setIsSubmitLocked] = React.useState(false);
  const onSendRef = React.useRef(onSend);
  const onEditSaveRef = React.useRef(onEditSave);
  const onEditLastOwnMessageRef = React.useRef(onEditLastOwnMessage);
  const editTargetRef = React.useRef(editTarget);
  const extractMentionPubkeysRef = React.useRef(mentions.extractMentionPubkeys);
  const ownerPubkeyRef = React.useRef(ownerPubkey);
  const syncAddressedAgentsFromTextRef = React.useRef<(text: string) => void>(
    () => {},
  );
  disabledRef.current = disabled;
  isSendingRef.current = isSending;
  isUploadingRef.current = media.isUploading;
  onSendRef.current = onSend;
  onEditSaveRef.current = onEditSave;
  onEditLastOwnMessageRef.current = onEditLastOwnMessage;
  editTargetRef.current = editTarget;
  extractMentionPubkeysRef.current = mentions.extractMentionPubkeys;
  ownerPubkeyRef.current = ownerPubkey;
  const isAutocompleteOpenRef = React.useRef(false);
  isAutocompleteOpenRef.current =
    mentions.isMentionOpen ||
    channelLinks.isChannelOpen ||
    emojiAutocomplete.isEmojiAutocompleteOpen;
  const submitMessageRef = React.useRef<() => void>(() => {});
  const composerScrollRef = React.useRef<HTMLDivElement>(null);
  const formRef = React.useRef<HTMLFormElement>(null);
  const composerOwnsFocus = useComposerFocusOwnership(formRef);
  const onEditLinkRef = React.useRef<
    ((info: LinkSelectionInfo) => void) | null
  >(null);
  const onLinkSelectionChangeRef = React.useRef<
    ((info: LinkSelectionInfo | null) => void) | null
  >(null);
  const onLinkShortcutRef = React.useRef<(() => boolean) | null>(null);
  const scrollComposerToBottom = React.useCallback(() => {
    window.requestAnimationFrame(() => {
      const scrollElement = composerScrollRef.current;
      if (!scrollElement) return;
      scrollElement.scrollTop = scrollElement.scrollHeight;
    });
  }, []);
  const computedPlaceholder = editTarget
    ? "Edit your message"
    : (placeholder ??
      (replyTarget
        ? `Reply to ${replyTarget.author} in #${channelName}`
        : `Message #${channelName}`));
  const richText = useRichTextEditor({
    placeholder: computedPlaceholder,
    editable: !composerDisabled,
    mentionNames: mentions.knownNames,
    agentMentionNames: mentions.agentKnownNames,
    channelNames: channelLinks.knownChannelNames,
    messageLinkChannels: channelLinks.channels,
    customEmoji,
    onSubmit: () => submitMessageRef.current(),
    onEditLastOwnMessage: () => {
      if (editTargetRef.current) return false;
      const handler = onEditLastOwnMessageRef.current;
      return handler ? handler() : false;
    },
    isAutocompleteOpen: isAutocompleteOpenRef,
    onEditLink: (info) => onEditLinkRef.current?.(info),
    onLinkSelectionChange: (info) => onLinkSelectionChangeRef.current?.(info),
    onLinkShortcut: () => onLinkShortcutRef.current?.() ?? false,
    onUpdate: ({ cursor, linkPreviewContent, text }) => {
      trackAuthoredContent(text);
      contentRef.current = text;
      setComposerContentFromText(text);
      setPreviewContent(linkPreviewContent);
      if (!isSubmitLockedRef.current && !editTargetRef.current) {
        syncAddressedAgentsFromTextRef.current(text);
      }
      mentions.updateMentionQuery(text, cursor);
      channelLinks.updateChannelQuery(text, cursor);
      emojiAutocomplete.updateEmojiQuery(text, cursor);
      if (text.trim().length > 0) {
        notifyTyping();
      }
    },
  });
  const linkEditor = useLinkEditor(richText);
  syncContentRefFromEditorRef.current = () => {
    const markdown = richText.getMarkdown();
    contentRef.current = markdown;
    return markdown;
  };
  onEditLinkRef.current = linkEditor.openFromClick;
  onLinkSelectionChangeRef.current = linkEditor.showFromCursor;
  onLinkShortcutRef.current = linkEditor.openFromShortcut;
  useComposerSpoilerParticles(richText.editor, composerScrollRef);
  const { audience: persistentAudience, keepMentionedAgentsPinned } =
    useThreadAgentAudience({
      isAgentPubkey: mentions.isAgentPubkey,
      rootTags: audienceContext?.rootTags ?? [],
      scope: audienceScope,
    });
  const addressPulse = useAddressMentionPulse();
  const {
    completeOptionsReveal: completeMentionOptionsReveal,
    confirmationTitle: autoPinConfirmationTitle,
    dismissConfirmation: dismissAutoPinConfirmation,
    openOptionsRequest: openMentionOptionsRequest,
    promoteExplicitlyAddressedAgents,
    promoteMentionedAgents,
    setConfirmationHovered: setAutoPinConfirmationHovered,
    turnOffConfirmation: turnOffAutoPinConfirmation,
  } = useAutoPinMentionedAgents({
    audienceScope,
    enabled: keepMentionedAgentsPinned,
    getDisplayName: mentions.getMentionDisplayName,
    onPulse: addressPulse.pulseOne,
    onTurnOff: () => setKeepMentionedAgentsPinned(false),
    onTurnOn: () => setKeepMentionedAgentsPinned(true),
  });
  const addressedMentionRestore = useAddressedAgentMentionRestore({
    audiencePubkeys: persistentAudience.pubkeys,
    channelId,
    enabled: keepMentionedAgentsPinned,
  });
  const mentionSendFlow = useMentionSendFlow({
    channelId,
    channelLinks,
    channelType,
    contentRef,
    customEmoji,
    drafts,
    emojiAutocomplete,
    mentions,
    onAddressedAgentsComposerCleared:
      addressedMentionRestore.onAddressedAgentsComposerCleared,
    onAddressedAgentsSendFailed: addressPulse.shakeMany,
    onAddressedAgentsSendSucceeded:
      addressedMentionRestore.onAddressedAgentsSendSucceeded,
    onPrepareSendChannel,
    onSendRef,
    richText,
    setContent: setComposerContent,
    setIsEmojiPickerOpen,
    setPendingImeta: media.setPendingImeta,
    hasUnsavedMedia: () =>
      media.pendingImetaRef.current.length > 0 ||
      media.queuedAttachmentsRef.current.length > 0,
    clearQueuedAttachments: media.clearQueuedAttachments,
    restoreQueuedAttachments: media.restoreQueuedAttachments,
    setSpoileredAttachmentUrls,
  });
  React.useEffect(() => {
    onDeferredEditPendingChange?.(isDeferredEditPending);
    return () => onDeferredEditPendingChange?.(false);
  }, [isDeferredEditPending, onDeferredEditPendingChange]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: editTarget?.id is the trigger
  React.useEffect(() => {
    if (editTarget && media.isUploading) return onCancelEdit?.();
    if (editTarget) {
      preEditSnapshotRef.current = {
        content: syncComposerContentFromEditor(),
        pendingImeta: [...media.pendingImetaRef.current],
        queuedAttachments: [...media.queuedAttachmentsRef.current],
        spoileredAttachmentUrls: new Set(spoileredAttachmentUrls),
      };
      const editableImeta = restoreImetaMediaDisplayLabels(
        editTarget.body,
        editTarget.imetaMedia ?? [],
      );
      const editableBody = stripImetaMediaLines(editTarget.body, editableImeta);
      setComposerContent(editableBody);
      richText.setContent(editableBody);
      mentions.restoreDraftMentionRefs(editTarget.mentionRefs ?? []);
      media.setPendingImeta(editableImeta);
      media.clearQueuedAttachments();
      setSpoileredAttachmentUrls(
        findSpoileredImetaMediaUrls(editTarget.body, editableImeta),
      );
      // also lands the caret at end of the loaded content.
      const rafId = requestAnimationFrame(() => richText.focusEnd());
      return () => cancelAnimationFrame(rafId);
    } else if (preEditSnapshotRef.current !== null) {
      const {
        content: restoredContent,
        pendingImeta: restoredImeta,
        queuedAttachments: restoredQueuedAttachments,
        spoileredAttachmentUrls: restoredSpoileredAttachmentUrls,
      } = preEditSnapshotRef.current;
      preEditSnapshotRef.current = null;
      setComposerContent(restoredContent);
      restoredContent
        ? richText.setContent(restoredContent)
        : richText.clearContent();
      media.setPendingImeta(restoredImeta);
      media.restoreQueuedAttachments(restoredQueuedAttachments);
      setSpoileredAttachmentUrls(restoredSpoileredAttachmentUrls);
    }
  }, [editTarget?.id]);
  React.useEffect(() => {
    if (!replyTarget || composerDisabled) return;
    richText.focusPreserve();
  }, [composerDisabled, replyTarget, richText.focusPreserve]);
  useComposerAutofocus(richText.focus, effectiveDraftKey, composerDisabled);
  const applyAutocompleteEdit = React.useCallback(
    (edit: AutocompleteEdit) => {
      richText.replacePlainTextRange(
        edit.replaceFromOffset,
        edit.replaceToOffset,
        edit.insertText,
        edit.customEmojiShortcode,
        edit.preserveSelection,
        edit.reassertMentionCaret,
      );
    },
    [richText.replacePlainTextRange],
  );
  const {
    announcement: addressLockAnnouncement,
    lockedAgents,
    lockedAgentPubkeys,
    removeAddressedAgent,
    restoreAddressedAgentMentions,
    selectMentionSuggestion,
    syncAddressedAgentsFromText,
    toggleAlwaysAddressAgent,
  } = useAgentAddressLockPicker({
    applyAutocompleteEdit,
    audience: persistentAudience,
    audienceScope,
    mentions,
    onAddressAgentMention: (suggestion) =>
      promoteExplicitlyAddressedAgents({
        pubkeys: suggestion.pubkey ? [suggestion.pubkey] : [],
      }),
    onAutoPinAgentMention: (suggestion, options) => {
      promoteMentionedAgents({
        ...options,
        pubkeys: suggestion.pubkey ? [suggestion.pubkey] : [],
      });
    },
    onImplicitPrefixInserted: implicitAgentMentionProvenance.add,
    onImplicitPrefixRemoved: implicitAgentMentionProvenance.remove,
    onPulseAddressLock: addressPulse.pulseOne,
    profiles,
    richText,
  });
  addressedMentionRestore.restoreAddressedAgentMentionsRef.current =
    restoreAddressedAgentMentions;
  React.useLayoutEffect(() => {
    if (!audienceScope || editTarget != null) return;
    restoreAddressedAgentMentions();
  }, [audienceScope, editTarget, restoreAddressedAgentMentions]);
  syncAddressedAgentsFromTextRef.current = syncAddressedAgentsFromText;
  const applyChannelInsert = React.useCallback(
    (suggestion: ChannelSuggestion) => {
      const { cursor } = richText.getPlainTextAndCursor();
      applyAutocompleteEdit(channelLinks.insertChannel(suggestion, cursor));
    },
    [
      applyAutocompleteEdit,
      channelLinks.insertChannel,
      richText.getPlainTextAndCursor,
    ],
  );
  const applyEmojiInsert = React.useCallback(
    (suggestion: EmojiSuggestion) => {
      const { cursor } = richText.getPlainTextAndCursor();
      applyAutocompleteEdit(emojiAutocomplete.insertEmoji(suggestion, cursor));
    },
    [
      applyAutocompleteEdit,
      emojiAutocomplete.insertEmoji,
      richText.getPlainTextAndCursor,
    ],
  );
  // ── Emoji insertion ─────────────────────────────────────────────────
  const insertEmoji = React.useCallback(
    (emoji: string) => {
      if (!richText.editor) return;
      const match = /^:([^:\s]+):$/.exec(emoji);
      const shortcode = match?.[1]?.toLowerCase();
      const known =
        shortcode &&
        customEmoji.some((e) => e.shortcode.toLowerCase() === shortcode);
      if (known && shortcode) {
        richText.editor
          .chain()
          .focus()
          .insertContent({
            type: CUSTOM_EMOJI_NODE_NAME,
            attrs: {
              shortcode,
              src:
                customEmoji.find((e) => e.shortcode.toLowerCase() === shortcode)
                  ?.url ?? "",
            },
          })
          .insertContent(" ")
          .run();
      } else {
        richText.editor.chain().focus().insertContent(emoji).run();
      }
      setIsEmojiPickerOpen(false);
      mentions.clearMentions();
    },
    [richText.editor, mentions.clearMentions, customEmoji],
  );
  const mentionPicker = useComposerMentionPicker({
    mentions,
    onTurnOffAutoPinConfirmation: turnOffAutoPinConfirmation,
    richText,
    setIsEmojiPickerOpen,
  });
  const handleAlwaysAddressShortcut = useAlwaysAddressShortcut({
    enabled: Boolean(audienceScope && editTarget == null),
    lockedAgent: lockedAgents[0],
    mentions,
    onOpenPicker: mentionPicker.openMentionPicker,
    onToggle: toggleAlwaysAddressAgent,
  });
  const submitMessage = React.useCallback(async () => {
    const trimmed = syncComposerContentFromEditor().trim();
    // Edit mode
    if (editTargetRef.current && onEditSaveRef.current) {
      // A live recording must be finished or discarded explicitly; never let an
      // edit save snapshot text while a voice note is mid-capture (the editor's
      // Enter shortcut bypasses the toolbar's Finish/Discard controls).
      if (isEditSubmissionLocked || voiceNote.statusRef.current !== "idle") {
        return;
      }
      // Empty edits delete the message through handleEditSave.
      await submitMessageEdit({
        content: trimmed,
        editTargetId: editTargetRef.current.id,
        customEmoji,
        originalContent: editTargetRef.current.body,
        ownerPubkey: ownerPubkeyRef.current,
        editTarget: editTargetRef.current,
        getMentionRefs: mentions.getDraftMentionRefs,
        pendingImeta: media.pendingImetaRef.current,
        queuedAttachments: media.queuedAttachmentsRef.current,
        spoileredAttachmentUrls,
        extractMentionPubkeys: extractMentionPubkeysRef.current,
        save: onEditSaveRef.current,
        clearComposer: () => {
          setComposerContent("");
          richText.clearContent();
          media.setPendingImeta([]);
          media.clearQueuedAttachments();
          setSpoileredAttachmentUrls(new Set());
          mentions.clearMentions();
          channelLinks.clearChannels();
          emojiAutocomplete.clearEmojis();
          setIsEmojiPickerOpen(false);
        },
        restoreComposer: (draft) => {
          setComposerContent(draft.content);
          richText.setContent(draft.content);
          media.setPendingImeta(draft.pendingImeta);
          media.restoreQueuedAttachments(draft.queuedAttachments);
          setSpoileredAttachmentUrls(draft.spoileredAttachmentUrls);
        },
        restoreMentionRefs: mentions.restoreDraftMentionRefs,
        revalidateMentionPubkeys: mentions.revalidateMentionPubkeys,
        shouldRestoreComposer: () => canRestoreEditDraftRef.current,
        setDeferredUploadPending: setDeferredEditPending,
        setUploadError: (message) =>
          media.setUploadState({ status: "error", message }),
      });
      return;
    }
    // Normal send
    const currentPendingImeta = media.pendingImetaRef.current;
    const currentQueuedAttachments = media.queuedAttachmentsRef.current;
    const hasMedia =
      currentPendingImeta.length > 0 || currentQueuedAttachments.length > 0;
    if (
      (!trimmed && !hasMedia) ||
      disabledRef.current ||
      voiceNote.statusRef.current !== "idle" ||
      isSendingRef.current ||
      isSubmitLockedRef.current ||
      isUploadingRef.current ||
      mentionSendFlow.isPreparingMentionSend
    ) {
      return;
    }
    const capturedThreadContext = onCaptureSendContext?.() ?? null;
    if (
      capturedThreadContext !== null &&
      !capturedThreadContext.parentEventId
    ) {
      return;
    }
    isSubmitLockedRef.current = true;
    setIsSubmitLocked(true);
    onPreparingMentionSendChange?.(true);
    try {
      const preparedLinkPreviews = getReadyLinkPreviewTags().some(
        (tag) => tag[1] === "none",
      )
        ? null
        : prepareBackgroundLinkPreviews(getLiveLinkPreviewCandidates());
      await mentionSendFlow.sendMessageWithMentionFlow({
        addressedAgentPubkeys: persistentAudience.pubkeys,
        capturedChannelId: channelId,
        capturedThreadContext,
        pendingImeta: currentPendingImeta,
        queuedAttachments: currentQueuedAttachments,
        linkPreviewTags: preparedLinkPreviews ? [] : getReadyLinkPreviewTags(),
        preparedLinkPreviews,
        sentDraftKey: resolveSentDraftKey(
          effectiveDraftKeyRef.current,
          drafts.loadDraft,
        ),
        recoveryDraftKey: effectiveDraftKey,
        spoileredAttachmentUrls,
        trimmed,
      });
    } finally {
      isSubmitLockedRef.current = false;
      setIsSubmitLocked(false);
      onPreparingMentionSendChange?.(false);
    }
  }, [
    channelId,
    channelLinks.clearChannels,
    customEmoji,
    drafts.loadDraft,
    emojiAutocomplete.clearEmojis,
    getLiveLinkPreviewCandidates,
    getReadyLinkPreviewTags,
    media.clearQueuedAttachments,
    media.pendingImetaRef,
    media.queuedAttachmentsRef,
    media.restoreQueuedAttachments,
    media.setPendingImeta,
    media.setUploadState,
    mentionSendFlow.isPreparingMentionSend,
    mentionSendFlow.sendMessageWithMentionFlow,
    mentions.clearMentions,
    richText.clearContent,
    richText.setContent,
    setComposerContent,
    setSpoileredAttachmentUrls,
    spoileredAttachmentUrls,
    syncComposerContentFromEditor,
    onCaptureSendContext,
    onPreparingMentionSendChange,
    persistentAudience.pubkeys,
    isEditSubmissionLocked,
    effectiveDraftKey,
    mentions.getDraftMentionRefs,
    mentions.restoreDraftMentionRefs,
    mentions.revalidateMentionPubkeys,
    voiceNote.statusRef,
  ]);
  submitMessageRef.current = submitMessage;
  // Draft auto-submit runs once after persisted editor state loads.
  const onAutoSubmitCompleteRef = React.useRef(onAutoSubmitComplete);
  onAutoSubmitCompleteRef.current = onAutoSubmitComplete;
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentionally fires once on mount only
  React.useEffect(() => {
    if (
      autoSubmitDraftKey === null ||
      autoSubmitDraftKey !== effectiveDraftKey
    ) {
      return;
    }
    // Clear the trigger BEFORE firing so any navigation from the send cannot
    // loop back with the param still present.
    onAutoSubmitCompleteRef.current?.();
    return scheduleSettleGatedAutoSubmit({
      submit: () => submitMessageRef.current(),
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // mount-only
  const handleSubmit = React.useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      void submitMessage();
    },
    [submitMessage],
  );
  // Tiptap handles formatting shortcuts (⌘B, ⌘I, etc.) natively.
  // Plain Enter → submit is now handled inside the Tiptap `submitOnEnter`
  // extension (fires before ProseMirror's splitBlock). This wrapper only
  // handles autocomplete arrow/enter keys and Escape for edit mode.
  const handleEditorKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (handleAlwaysAddressShortcut(event)) return;
      // Let autocomplete handle keys first
      const emojiResult = emojiAutocomplete.handleEmojiKeyDown(event);
      if (emojiResult.handled) {
        if (emojiResult.suggestion) {
          applyEmojiInsert(emojiResult.suggestion);
        }
        return;
      }
      const channelResult = channelLinks.handleChannelKeyDown(event);
      if (channelResult.handled) {
        if (channelResult.suggestion) {
          applyChannelInsert(channelResult.suggestion);
        }
        return;
      }
      // Shift+Tab is the keyboard route from the editor into the mention
      // overlay's Options controls — forward Tab is consumed below to select
      // the highlighted suggestion. Falls through (e.g. a composer with no
      // audience controls) to the browser's native backward focus move.
      if (
        event.key === "Tab" &&
        event.shiftKey &&
        mentions.isMentionOpen &&
        focusMentionOptionsTrigger(formRef.current)
      ) {
        event.preventDefault();
        return;
      }
      const { handled, suggestion } = mentions.handleMentionKeyDown(event, {
        isCodeContext: () => isMentionCodeContext(richText.editor),
      });
      if (handled) {
        if (suggestion) {
          selectMentionSuggestion(suggestion);
        }
        return;
      }
      if (event.key === "Tab" && !event.shiftKey && linkEditor.isCardOpen) {
        event.preventDefault();
        if (!linkEditor.focusCardFirstControl()) {
          requestAnimationFrame(linkEditor.focusCardFirstControl);
        }
        return;
      }
      // Escape in edit mode
      if (
        event.key === "Escape" &&
        !isDeferredEditPending &&
        editTargetRef.current &&
        onCancelEdit
      ) {
        event.preventDefault();
        onCancelEdit();
        return;
      }
    },
    [
      handleAlwaysAddressShortcut,
      emojiAutocomplete.handleEmojiKeyDown,
      applyEmojiInsert,
      channelLinks.handleChannelKeyDown,
      applyChannelInsert,
      mentions.isMentionOpen,
      mentions.handleMentionKeyDown,
      richText.editor,
      selectMentionSuggestion,
      linkEditor.isCardOpen,
      linkEditor.focusCardFirstControl,
      isDeferredEditPending,
      onCancelEdit,
    ],
  );
  useComposerPasteHandler({
    editor: richText.editor,
    scrollToBottom: scrollComposerToBottom,
    setPendingImeta: voiceNote.setPendingImetaWhenIdle,
    uploadFile: voiceNote.uploadFileWhenIdle,
  });
  const sendDisabled =
    composerDisabled ||
    media.isUploading ||
    voiceNote.status !== "idle" ||
    mentionSendFlow.isPreparingMentionSend ||
    (isContentEmpty &&
      media.pendingImeta.length === 0 &&
      media.queuedAttachments.length === 0);
  const handleCaptureSelection = React.useCallback(() => {}, []);
  const handlePaperclipClick = React.useCallback(() => {
    if (!voiceNote.hasAttachmentRef.current) void media.handlePaperclip();
  }, [media.handlePaperclip, voiceNote.hasAttachmentRef]);
  const acceptsDrop = ownsDropZone && voiceNote.acceptsAttachment;
  return (
    <>
      <footer
        className={cn(
          "relative z-10 shrink-0 bg-transparent px-4 pb-2 pt-0",
          showTopBorder ? "border-t border-border/40 pt-3" : "",
          containerClassName,
        )}
      >
        <div
          aria-hidden="true"
          className="absolute inset-x-0 bottom-0 h-5 bg-transparent"
        />
        <div className="relative flex w-full flex-col gap-0">
          <ComposerReplyEditBanner
            isEditing={editTarget != null}
            isEditCancelDisabled={isDeferredEditPending}
            replyTarget={replyTarget}
            onCancelEdit={onCancelEdit}
            onCancelReply={onCancelReply}
          />
          {showBackgroundUploadProgress ? (
            <ComposerUploadProgressPill
              canCancel={backgroundUpload.canCancel}
              isUploading={backgroundUpload.isUploading}
              onCancel={cancelBackgroundMediaUploads}
              phase={backgroundUpload.phase}
              percentage={backgroundUpload.percentage}
            />
          ) : null}
          <form
            className={cn(
              "relative z-10 isolate rounded-2xl border border-border/50 bg-background/80 px-3 pb-2 pt-3 shadow-none supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:supports-[backdrop-filter]:bg-background/55 sm:px-4",
              layoutMode === "standalone" &&
                "backdrop-blur-md dark:backdrop-blur-xl",
            )}
            data-submit-locked={isSubmitLocked ? "true" : "false"}
            data-testid="message-composer"
            onDragEnter={acceptsDrop ? media.handleDragEnter : undefined}
            onDragLeave={acceptsDrop ? media.handleDragLeave : undefined}
            onDragOver={acceptsDrop ? media.handleDragOver : undefined}
            onDrop={
              acceptsDrop
                ? (e) => {
                    if (isDeferredEditPending) {
                      e.preventDefault();
                      return;
                    }
                    void media.handleDrop(e);
                  }
                : undefined
            }
            onSubmit={(event) => {
              handleSubmit(event);
            }}
            ref={formRef}
          >
            {acceptsDrop && media.isDragOver && <DropZoneOverlay />}
            <MessageComposerAutocompletes
              audienceControlsEnabled={Boolean(
                audienceScope && editTarget == null,
              )}
              channelLinks={channelLinks}
              composerOwnsFocus={composerOwnsFocus}
              emojiAutocomplete={emojiAutocomplete}
              keepMentionedAgentsPinned={keepMentionedAgentsPinned}
              lockedAgentPubkeys={lockedAgentPubkeys}
              mentions={mentions}
              openOptionsRequest={openMentionOptionsRequest}
              onChannelSelect={applyChannelInsert}
              onEmojiSelect={applyEmojiInsert}
              onMentionSelect={selectMentionSuggestion}
              onOptionsRevealComplete={completeMentionOptionsReveal}
              onToggleAlwaysAddressAgent={(suggestion) =>
                toggleAlwaysAddressAgent(suggestion, {
                  preserveMention: true,
                })
              }
            />
            {media.uploadState.status === "error" ? (
              <div className="mb-2 rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
                Upload failed: {media.uploadState.message}
                <button
                  className="ml-2 underline"
                  onClick={() => media.setUploadState({ status: "idle" })}
                  type="button"
                >
                  Dismiss
                </button>
              </div>
            ) : null}
            {composerLinkPreviews}
            <output
              aria-live="polite"
              className="sr-only"
              data-testid="composer-address-lock-status"
            >
              {addressLockAnnouncement}
            </output>
            {(media.pendingImeta.length > 0 ||
              media.queuedAttachments.length > 0 ||
              media.isUploading) && (
              <div className="mb-2 flex flex-wrap items-center gap-2">
                {media.pendingImeta.length > 0 ||
                media.queuedAttachments.length > 0 ||
                media.isUploading ? (
                  <ComposerAttachments
                    attachments={media.pendingImeta}
                    isUploading={media.isUploading}
                    onCancelUpload={media.cancelUpload}
                    onRemoveQueued={media.removeQueuedAttachment}
                    onToggleQueuedSpoiler={media.toggleQueuedAttachmentSpoiler}
                    queuedPreviews={media.queuedPreviews}
                    uploadingCount={media.uploadingCount}
                    uploadingPreviews={media.uploadingPreviews}
                    onEditSave={handleAttachmentEditSave}
                    onRemove={handleRemoveAttachment}
                    onRevert={handleAttachmentRevert}
                    originalUrlByUrl={media.originalUrlByUrl}
                    onToggleSpoiler={handleToggleAttachmentSpoiler}
                    spoileredUrls={spoileredAttachmentUrls}
                  />
                ) : null}
              </div>
            )}
            {/* biome-ignore lint/a11y/noStaticElementInteractions: keydown handler bridges Tiptap editor to autocomplete and submit */}
            <div
              className="rich-text-composer relative max-h-32 overflow-y-auto"
              data-testid="message-input-scroll"
              ref={composerScrollRef}
              onKeyDown={handleEditorKeyDown}
            >
              <EditorContent editor={richText.editor} />
            </div>
            <ComposerDockToolbar
              addressedAgents={
                editTarget == null && !composerDisabled ? lockedAgents : []
              }
              autoPinConfirmationTitle={autoPinConfirmationTitle}
              layoutMode={layoutMode}
              composerDisabled={composerDisabled}
              editor={richText.editor}
              extraActions={toolbarExtraActions}
              formattingDisabled={composerDisabled}
              gifMediaController={media}
              isEmojiPickerOpen={isEmojiPickerOpen}
              isFormattingOpen={isFormattingOpen}
              isSending={isSending || mentionSendFlow.isPreparingMentionSend}
              isUploading={media.isUploading}
              isVoiceNoteProcessing={voiceNote.status !== "recording"}
              isVoiceNoteRecording={voiceNote.status !== "idle"}
              hasVoiceNoteAttachment={voiceNote.hasAttachment}
              voiceNoteRecorder={voiceNote.recorderElement}
              onCaptureSelection={handleCaptureSelection}
              onAutoPinConfirmationDismiss={dismissAutoPinConfirmation}
              onAutoPinConfirmationHoverChange={setAutoPinConfirmationHovered}
              onAutoPinConfirmationTurnOff={mentionPicker.turnOff}
              onEmojiPickerOpenChange={setIsEmojiPickerOpen}
              onEmojiSelect={insertEmoji}
              onFormattingToggle={handleFormattingToggle}
              onLinkButton={linkEditor.openFromToolbar}
              onOpenMentionPicker={mentionPicker.openMentionSettings}
              onPaperclip={handlePaperclipClick}
              onFinishVoiceNote={() => void voiceNote.finish()}
              onVoiceNote={voiceNote.toggle}
              onRemoveAddressedAgent={removeAddressedAgent}
              pulseVersionByPubkey={addressPulse.pulseVersionByPubkey}
              sendDisabled={sendDisabled}
              shakeVersionByPubkey={addressPulse.shakeVersionByPubkey}
            />
          </form>
        </div>
      </footer>
      <NonMemberMentionDialog {...mentionSendFlow.nonMemberPromptProps} />
      {linkEditor.card}
      {linkEditor.dialog}
    </>
  );
}
export const MessageComposer = React.memo(MessageComposerImpl);
