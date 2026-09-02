import * as React from "react";

import { EditorContent } from "@tiptap/react";
import { ChevronDown } from "lucide-react";
import { buildOutgoingMessage } from "@/features/messages/lib/imetaMediaMarkdown";
import { useChannelLinks } from "@/features/messages/lib/useChannelLinks";
import type { ChannelSuggestion } from "@/features/messages/lib/useChannelLinks";
import { useComposerFocusOwnership } from "@/features/messages/lib/useComposerFocusOwnership";
import { useMediaUpload } from "@/features/messages/lib/useMediaUpload";
import { isMentionCodeContext } from "@/features/messages/lib/mentionCodeContext";
import { useMentions } from "@/features/messages/lib/useMentions";
import {
  hasMentionClipboardHtml,
  normalizeMentionClipboardHtml,
} from "@/features/messages/lib/normalizeMentionClipboard";
import {
  type LinkSelectionInfo,
  useRichTextEditor,
} from "@/features/messages/lib/useRichTextEditor";
import { useLinkEditor } from "@/features/messages/lib/useLinkEditor";
import { DropZoneOverlay } from "@/features/messages/ui/ComposerAttachments";
import type { MentionSuggestion } from "@/features/messages/ui/MentionAutocomplete";
import { MessageComposerToolbar } from "@/features/messages/ui/MessageComposerToolbar";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import type { ForumComposerProps } from "./ForumComposer.types";
import { ForumComposerAutocompletes } from "./ForumComposerAutocompletes";
import { ForumComposerCompactLayout } from "./ForumComposerCompactLayout";
import { ForumComposerMediaStatus } from "./ForumComposerMediaStatus";
import { useCompactComposerInteractions } from "./useCompactComposerInteractions";

export function ForumComposer({
  channelId = null,
  channelType,
  members,
  className,
  placeholder,
  disabled,
  header,
  isSending,
  onCancel,
  onSecondarySubmit,
  onSubmit,
  secondarySubmitLabel,
  compact = false,
  autocompleteBelow = false,
  profiles,
}: ForumComposerProps) {
  const [content, setContent] = React.useState("");
  const contentRef = React.useRef(content);
  contentRef.current = content;

  const [isCompactExpanded, setIsCompactExpanded] = React.useState(!compact);
  const [isEmojiPickerOpen, setIsEmojiPickerOpen] = React.useState(false);
  const [isFormattingOpen, setIsFormattingOpen] = React.useState(false);
  const [isSubmissionPending, setIsSubmissionPending] = React.useState(false);
  const [submitMode, setSubmitMode] = React.useState<"primary" | "secondary">(
    "primary",
  );

  const handleFormattingToggle = React.useCallback((pressed: boolean) => {
    if (pressed) setIsEmojiPickerOpen(false);
    setIsFormattingOpen(pressed);
  }, []);
  const expandCompactComposer = React.useCallback(() => {
    if (compact) setIsCompactExpanded(true);
  }, [compact]);

  const mentions = useMentions(channelId, members, profiles, { channelType });
  const channelLinks = useChannelLinks();
  const media = useMediaUpload();
  const { handlePaperclipClick, handleToolbarMouseDown, shouldIgnoreBlur } =
    useCompactComposerInteractions({
      compact,
      onExpand: expandCompactComposer,
      onPaperclip: media.handlePaperclip,
    });

  const disabledRef = React.useRef(disabled);
  const isSendingRef = React.useRef(isSending);
  const isUploadingRef = React.useRef(media.isUploading);
  const isSubmissionPendingRef = React.useRef(false);
  const onSubmitRef = React.useRef(onSubmit);
  const onSecondarySubmitRef = React.useRef(onSecondarySubmit);
  const submitModeRef = React.useRef(submitMode);
  disabledRef.current = disabled;
  isSendingRef.current = isSending;
  isUploadingRef.current = media.isUploading;
  onSubmitRef.current = onSubmit;
  onSecondarySubmitRef.current = onSecondarySubmit;
  submitModeRef.current = onSecondarySubmit ? submitMode : "primary";

  const isAutocompleteOpenRef = React.useRef(false);
  isAutocompleteOpenRef.current =
    mentions.isMentionOpen || channelLinks.isChannelOpen;

  const submitMessageRef = React.useRef<() => void>(() => {});
  const formRef = React.useRef<HTMLFormElement>(null);
  const composerOwnsFocus = useComposerFocusOwnership(formRef);

  // Set after `useLinkEditor` exists; the editor's link-click handler
  // delegates through this ref to break the hook ordering cycle.
  const onEditLinkRef = React.useRef<
    ((info: LinkSelectionInfo) => void) | null
  >(null);
  const onLinkSelectionChangeRef = React.useRef<
    ((info: LinkSelectionInfo | null) => void) | null
  >(null);
  const onLinkShortcutRef = React.useRef<(() => boolean) | null>(null);

  const richText = useRichTextEditor({
    placeholder,
    editable: !disabled && !isSubmissionPending,
    mentionNames: mentions.knownNames,
    channelNames: channelLinks.knownChannelNames,
    messageLinkChannels: channelLinks.channels,
    onSubmit: () => submitMessageRef.current(),
    isAutocompleteOpen: isAutocompleteOpenRef,
    onEditLink: (info) => onEditLinkRef.current?.(info),
    onLinkSelectionChange: (info) => onLinkSelectionChangeRef.current?.(info),
    onLinkShortcut: () => onLinkShortcutRef.current?.() ?? false,
    onUpdate: ({ cursor, text }) => {
      const markdown = richText.getMarkdown();
      setContent(markdown);
      contentRef.current = markdown;

      mentions.updateMentionQuery(text, cursor);
      channelLinks.updateChannelQuery(text, cursor);
    },
  });

  const linkEditor = useLinkEditor(richText);
  onEditLinkRef.current = linkEditor.openFromClick;
  onLinkSelectionChangeRef.current = linkEditor.showFromCursor;
  onLinkShortcutRef.current = linkEditor.openFromShortcut;

  // ── Mention / channel autocomplete insertion ────────────────────────
  // Native ProseMirror transactions — no markdown round-trip.
  const applyMentionInsert = React.useCallback(
    (suggestion: MentionSuggestion) => {
      if (isSubmissionPendingRef.current) return;
      const { cursor } = richText.getPlainTextAndCursor();
      const { replaceFromOffset, replaceToOffset, insertText } =
        mentions.insertMention(suggestion, cursor);
      richText.replacePlainTextRange(
        replaceFromOffset,
        replaceToOffset,
        insertText,
      );
    },
    [
      mentions.insertMention,
      richText.getPlainTextAndCursor,
      richText.replacePlainTextRange,
    ],
  );

  const applyChannelInsert = React.useCallback(
    (suggestion: ChannelSuggestion) => {
      if (isSubmissionPendingRef.current) return;
      const { cursor } = richText.getPlainTextAndCursor();
      const { replaceFromOffset, replaceToOffset, insertText } =
        channelLinks.insertChannel(suggestion, cursor);
      richText.replacePlainTextRange(
        replaceFromOffset,
        replaceToOffset,
        insertText,
      );
    },
    [
      channelLinks.insertChannel,
      richText.getPlainTextAndCursor,
      richText.replacePlainTextRange,
    ],
  );

  const insertEmoji = React.useCallback(
    (emoji: string) => {
      if (isSubmissionPendingRef.current || !richText.editor) return;
      richText.editor.chain().focus().insertContent(emoji).run();
      setIsEmojiPickerOpen(false);
      mentions.clearMentions();
    },
    [richText.editor, mentions.clearMentions],
  );

  // ── @ mention picker (toolbar button) ───────────────────────────────
  const openMentionPicker = React.useCallback(() => {
    if (!richText.editor) return;
    const { text, cursor } = richText.getPlainTextAndCursor();

    const beforeCursor = text.slice(0, cursor);
    if (/(?:^|[\s])@[^\s]*$/.test(beforeCursor)) {
      mentions.updateMentionQuery(text, cursor);
      richText.focus();
      return;
    }

    const previousChar = text.slice(0, cursor).slice(-1);
    const prefix =
      cursor > 0 && previousChar && !/\s/.test(previousChar) ? " @" : "@";
    richText.editor.chain().focus().insertContent(prefix).run();
    setIsEmojiPickerOpen(false);

    const { text: updatedText, cursor: updatedCursor } =
      richText.getPlainTextAndCursor();
    mentions.updateMentionQuery(updatedText, updatedCursor);
  }, [
    richText.editor,
    richText.getPlainTextAndCursor,
    richText.focus,
    mentions.updateMentionQuery,
  ]);

  // ── Submit ──────────────────────────────────────────────────────────
  const submitMessage = React.useCallback(
    async (submitter = onSubmitRef.current) => {
      const trimmed = contentRef.current.trim();
      const currentPendingImeta = media.pendingImetaRef.current;
      const hasMedia = currentPendingImeta.length > 0;

      if (
        (!trimmed && !hasMedia) ||
        disabledRef.current ||
        isSendingRef.current ||
        isUploadingRef.current ||
        isSubmissionPendingRef.current
      ) {
        return;
      }

      isSubmissionPendingRef.current = true;
      setIsSubmissionPending(true);
      mentions.cancelMentionAutocomplete();
      channelLinks.clearChannels();
      setIsEmojiPickerOpen(false);
      try {
        const pubkeys = await mentions.revalidateMentionPubkeys(
          mentions.extractMentionPubkeys(trimmed),
        );

        // Reuse the shared send-path builder so forum/notes posts emit the same
        // body + imeta as chat: generic files become `[filename](url)` links with a
        // `filename` imeta tag (FileCard renderer), images/video stay inline. Send
        // semantics use `undefined` for "no attachments" (no imeta tags emitted).
        const { content: finalContent, mediaTags } = buildOutgoingMessage(
          trimmed,
          currentPendingImeta,
        );

        // Save draft state so we can restore on failure.
        const savedContent = contentRef.current;
        const savedImeta = [...currentPendingImeta];

        setContent("");
        contentRef.current = "";
        richText.clearContent();
        media.setPendingImeta([]);
        mentions.clearMentions();
        channelLinks.clearChannels();
        setIsEmojiPickerOpen(false);

        try {
          await submitter(finalContent, pubkeys, mediaTags);
          setSubmitMode("primary");
          if (compact) setIsCompactExpanded(false);
        } catch {
          setContent(savedContent);
          contentRef.current = savedContent;
          richText.setContent(savedContent);
          media.setPendingImeta(savedImeta);
          if (compact) setIsCompactExpanded(true);
        }
      } catch {
        // Keep the draft intact when authorization refresh fails.
      } finally {
        isSubmissionPendingRef.current = false;
        setIsSubmissionPending(false);
      }
    },
    [
      compact,
      media.pendingImetaRef,
      media.setPendingImeta,
      mentions.cancelMentionAutocomplete,
      mentions.extractMentionPubkeys,
      mentions.revalidateMentionPubkeys,
      mentions.clearMentions,
      channelLinks.clearChannels,
      richText.clearContent,
      richText.setContent,
    ],
  );
  const submitSelectedMessage = React.useCallback(() => {
    const secondarySubmit = onSecondarySubmitRef.current;
    submitMessage(
      submitModeRef.current === "secondary" && secondarySubmit
        ? secondarySubmit
        : onSubmitRef.current,
    );
  }, [submitMessage]);
  submitMessageRef.current = submitSelectedMessage;

  const handleSubmit = React.useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      submitSelectedMessage();
    },
    [submitSelectedMessage],
  );

  // ── Keyboard handling ───────────────────────────────────────────────
  const handleEditorKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      const channelResult = channelLinks.handleChannelKeyDown(event);
      if (channelResult.handled) {
        if (channelResult.suggestion) {
          applyChannelInsert(channelResult.suggestion);
        }
        return;
      }

      const { handled, suggestion } = mentions.handleMentionKeyDown(event, {
        isCodeContext: () => isMentionCodeContext(richText.editor),
      });
      if (handled) {
        if (suggestion) {
          applyMentionInsert(suggestion);
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
    },
    [
      channelLinks.handleChannelKeyDown,
      applyChannelInsert,
      mentions.handleMentionKeyDown,
      richText.editor,
      applyMentionInsert,
      linkEditor.isCardOpen,
      linkEditor.focusCardFirstControl,
    ],
  );

  // ── Media paste ─────────────────────────────────────────────────────
  const uploadFileRef = React.useRef(media.uploadFile);
  uploadFileRef.current = media.uploadFile;

  React.useEffect(() => {
    if (!richText.editor) return;

    richText.editor.setOptions({
      editorProps: {
        ...richText.editor.options.editorProps,
        handlePaste: (_view, event) => {
          const items = Array.from(event.clipboardData?.items ?? []);
          // Any actual file pastes as an attachment; text/string items fall
          // through to the handlers below.
          const mediaItem = items.find((item) => item.kind === "file");
          if (mediaItem) {
            const file = mediaItem.getAsFile();
            if (file) {
              void uploadFileRef.current(file);
            }
            return true;
          }

          const html = event.clipboardData?.getData("text/html");
          if (html && hasMentionClipboardHtml(html)) {
            const cleanHtml = normalizeMentionClipboardHtml(html);
            event.preventDefault();
            _view.pasteHTML(cleanHtml);
            return true;
          }

          return false;
        },
      },
    });
  }, [richText.editor]);

  const sendDisabled = React.useMemo(
    () =>
      disabled ||
      isSubmissionPending ||
      media.isUploading ||
      (content.trim().length === 0 && media.pendingImeta.length === 0),
    [
      disabled,
      isSubmissionPending,
      media.isUploading,
      content,
      media.pendingImeta.length,
    ],
  );
  const hasComposerContent =
    content.trim().length > 0 ||
    media.pendingImeta.length > 0 ||
    media.isUploading ||
    media.uploadState.status === "error";
  const isExpanded =
    !compact ||
    isCompactExpanded ||
    hasComposerContent ||
    isEmojiPickerOpen ||
    isFormattingOpen ||
    mentions.isMentionOpen ||
    channelLinks.isChannelOpen;
  const isCompactLayout = compact && !isExpanded;
  const handleFormBlur = React.useCallback(
    (event: React.FocusEvent<HTMLFormElement>) => {
      if (!compact) return;

      const nextTarget = event.relatedTarget;
      if (
        nextTarget instanceof Node &&
        event.currentTarget.contains(nextTarget)
      ) {
        return;
      }
      if (shouldIgnoreBlur()) {
        return;
      }

      const hasDraft =
        contentRef.current.trim().length > 0 ||
        media.pendingImetaRef.current.length > 0 ||
        media.isUploading ||
        media.uploadState.status === "error" ||
        isEmojiPickerOpen ||
        isFormattingOpen;

      if (!hasDraft) setIsCompactExpanded(false);
    },
    [
      compact,
      isEmojiPickerOpen,
      isFormattingOpen,
      media.isUploading,
      media.pendingImetaRef,
      media.uploadState.status,
      shouldIgnoreBlur,
    ],
  );
  const wasCompactExpandedRef = React.useRef(isCompactExpanded);
  React.useEffect(() => {
    const wasExpanded = wasCompactExpandedRef.current;
    wasCompactExpandedRef.current = isCompactExpanded;

    if (!compact || !isCompactExpanded || wasExpanded) return;

    const frame = window.requestAnimationFrame(() => {
      richText.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [compact, isCompactExpanded, richText.focus]);
  const autocompletePosition = autocompleteBelow ? "below" : "above";
  return (
    <>
      <form
        className={cn(
          "relative rounded-2xl border border-input bg-card px-3 py-2 sm:px-4",
          className,
        )}
        inert={isSubmissionPending ? true : undefined}
        onBlurCapture={handleFormBlur}
        onDragEnter={(event) => {
          if (isSubmissionPending) {
            event.preventDefault();
            return;
          }
          expandCompactComposer();
          media.handleDragEnter(event);
        }}
        onDragLeave={media.handleDragLeave}
        onDragOver={(event) => {
          if (isSubmissionPending) {
            event.preventDefault();
            return;
          }
          media.handleDragOver(event);
        }}
        onDrop={(event) => {
          if (isSubmissionPending) {
            event.preventDefault();
            return;
          }
          void media.handleDrop(event);
        }}
        onFocusCapture={expandCompactComposer}
        onSubmit={handleSubmit}
        ref={formRef}
      >
        {media.isDragOver && <DropZoneOverlay />}
        {isCompactLayout ? (
          <ForumComposerCompactLayout
            editor={richText.editor}
            header={header}
            isSending={Boolean(isSending || isSubmissionPending)}
            onEditorKeyDown={handleEditorKeyDown}
            sendDisabled={sendDisabled}
          />
        ) : (
          <>
            {header ? (
              <div
                className={cn("mb-2", compact && "flex min-h-10 items-center")}
              >
                {header}
              </div>
            ) : null}
            <ForumComposerAutocompletes
              channelSelectedIndex={channelLinks.channelSelectedIndex}
              channelSuggestions={
                channelLinks.isChannelOpen
                  ? channelLinks.channelSuggestions
                  : []
              }
              composerOwnsFocus={composerOwnsFocus}
              mentionSelectedIndex={mentions.mentionSelectedIndex}
              mentionSuggestions={
                mentions.isMentionOpen ? mentions.suggestions : []
              }
              onChannelSelect={applyChannelInsert}
              onMentionDismiss={mentions.cancelMentionAutocomplete}
              onMentionFetchMore={mentions.fetchMoreSuggestions}
              onMentionSelect={applyMentionInsert}
              position={autocompletePosition}
            />

            <fieldset
              className="min-w-0 border-0 p-0"
              disabled={isSubmissionPending}
            >
              <ForumComposerMediaStatus
                disabled={isSubmissionPending}
                media={media}
              />
            </fieldset>

            {/* biome-ignore lint/a11y/noStaticElementInteractions: keydown handler bridges Tiptap editor to autocomplete and submit */}
            <div
              className="rich-text-composer max-h-32 overflow-y-auto"
              onKeyDown={handleEditorKeyDown}
            >
              <EditorContent editor={richText.editor} />
            </div>

            <MessageComposerToolbar
              composerDisabled={Boolean(disabled || isSubmissionPending)}
              editor={richText.editor}
              extraActions={
                onCancel || (onSecondarySubmit && secondarySubmitLabel) ? (
                  <>
                    {onCancel ? (
                      <Button
                        disabled={isSending || isSubmissionPending}
                        onClick={onCancel}
                        size="sm"
                        type="button"
                        variant="ghost"
                      >
                        Cancel
                      </Button>
                    ) : null}
                    {onSecondarySubmit && secondarySubmitLabel ? (
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button
                            className={cn(
                              submitMode === "secondary" &&
                                "border-amber-500/40 text-amber-700 hover:bg-amber-500/10 hover:text-amber-800 dark:text-amber-400 dark:hover:text-amber-300",
                            )}
                            disabled={
                              disabled || isSending || isSubmissionPending
                            }
                            size="sm"
                            type="button"
                            variant="outline"
                          >
                            {submitMode === "secondary"
                              ? secondarySubmitLabel
                              : "Comment"}
                            <ChevronDown className="h-3.5 w-3.5" />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuRadioGroup
                            onValueChange={(value) =>
                              setSubmitMode(value as "primary" | "secondary")
                            }
                            value={submitMode}
                          >
                            <DropdownMenuRadioItem value="primary">
                              Comment
                            </DropdownMenuRadioItem>
                            <DropdownMenuRadioItem value="secondary">
                              {secondarySubmitLabel}
                            </DropdownMenuRadioItem>
                          </DropdownMenuRadioGroup>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    ) : null}
                  </>
                ) : undefined
              }
              formattingDisabled={Boolean(disabled || isSubmissionPending)}
              gifMediaController={media}
              isEmojiPickerOpen={isEmojiPickerOpen}
              isFormattingOpen={isFormattingOpen}
              isSending={Boolean(isSending || isSubmissionPending)}
              isUploading={media.isUploading}
              onCaptureSelection={handleToolbarMouseDown}
              onEmojiPickerOpenChange={setIsEmojiPickerOpen}
              onEmojiSelect={insertEmoji}
              onFormattingToggle={handleFormattingToggle}
              onLinkButton={linkEditor.openFromToolbar}
              onOpenMentionPicker={openMentionPicker}
              onPaperclip={handlePaperclipClick}
              sendDisabled={sendDisabled}
            />
          </>
        )}
      </form>
      {!isSubmissionPending && linkEditor.card}
      {!isSubmissionPending && linkEditor.dialog}
    </>
  );
}
