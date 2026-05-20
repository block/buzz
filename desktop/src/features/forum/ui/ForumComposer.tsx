import * as React from "react";

import { EditorContent } from "@tiptap/react";
import { useChannelLinks } from "@/features/messages/lib/useChannelLinks";
import type { ChannelSuggestion } from "@/features/messages/lib/useChannelLinks";
import {
  ALLOWED_MEDIA_TYPES,
  useMediaUpload,
} from "@/features/messages/lib/useMediaUpload";
import { useMentions } from "@/features/messages/lib/useMentions";
import {
  hasMentionClipboardHtml,
  normalizeMentionClipboardHtml,
} from "@/features/messages/lib/normalizeMentionClipboard";
import { useRichTextEditor } from "@/features/messages/lib/useRichTextEditor";
import { DropZoneOverlay } from "@/features/messages/ui/ComposerAttachments";
import type { MentionSuggestion } from "@/features/messages/ui/MentionAutocomplete";
import { MessageComposerToolbar } from "@/features/messages/ui/MessageComposerToolbar";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import type { ForumComposerProps } from "./ForumComposer.types";
import { ForumComposerAutocompletes } from "./ForumComposerAutocompletes";
import { ForumComposerCompactLayout } from "./ForumComposerCompactLayout";
import { ForumComposerMediaStatus } from "./ForumComposerMediaStatus";
import { useCompactComposerInteractions } from "./useCompactComposerInteractions";

export function ForumComposer({
  channelId = null,
  members,
  className,
  placeholder,
  disabled,
  header,
  isSending,
  onCancel,
  onSubmit,
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

  const handleFormattingToggle = React.useCallback((pressed: boolean) => {
    if (pressed) setIsEmojiPickerOpen(false);
    setIsFormattingOpen(pressed);
  }, []);
  const expandCompactComposer = React.useCallback(() => {
    if (compact) setIsCompactExpanded(true);
  }, [compact]);

  const mentions = useMentions(channelId, members, profiles);
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
  const onSubmitRef = React.useRef(onSubmit);
  disabledRef.current = disabled;
  isSendingRef.current = isSending;
  onSubmitRef.current = onSubmit;

  const isAutocompleteOpenRef = React.useRef(false);
  isAutocompleteOpenRef.current =
    mentions.isMentionOpen || channelLinks.isChannelOpen;

  const submitMessageRef = React.useRef<() => void>(() => {});

  const richText = useRichTextEditor({
    placeholder,
    editable: !disabled,
    mentionNames: mentions.knownNames,
    channelNames: channelLinks.knownChannelNames,
    onSubmit: () => submitMessageRef.current(),
    isAutocompleteOpen: isAutocompleteOpenRef,
    onUpdate: ({ markdown, text }) => {
      setContent(markdown);
      contentRef.current = markdown;

      const { cursor } = richText.getPlainTextAndCursor();
      mentions.updateMentionQuery(text, cursor);
      channelLinks.updateChannelQuery(text, cursor);
    },
  });

  // ── Mention / channel autocomplete insertion ────────────────────────
  // Native ProseMirror transactions — no markdown round-trip.
  const applyMentionInsert = React.useCallback(
    (suggestion: MentionSuggestion) => {
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
      if (!richText.editor) return;
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
  const submitMessage = React.useCallback(() => {
    const trimmed = contentRef.current.trim();
    const currentPendingImeta = media.pendingImetaRef.current;
    const hasMedia = currentPendingImeta.length > 0;

    if (
      (!trimmed && !hasMedia) ||
      disabledRef.current ||
      isSendingRef.current
    ) {
      return;
    }

    const pubkeys = mentions.extractMentionPubkeys(trimmed);

    const mediaTags =
      currentPendingImeta.length > 0
        ? currentPendingImeta.map((d) => [
            "imeta",
            `url ${d.url}`,
            `m ${d.type}`,
            `x ${d.sha256}`,
            `size ${d.size}`,
            ...(d.dim ? [`dim ${d.dim}`] : []),
            ...(d.blurhash ? [`blurhash ${d.blurhash}`] : []),
            ...(d.thumb ? [`thumb ${d.thumb}`] : []),
            ...(d.duration != null ? [`duration ${d.duration}`] : []),
            ...(d.image ? [`image ${d.image}`] : []),
          ])
        : undefined;

    let finalContent = trimmed;
    for (const d of currentPendingImeta) {
      const isVideo = d.type.startsWith("video/");
      finalContent += isVideo ? `\n![video](${d.url})` : `\n![image](${d.url})`;
    }

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

    const result = onSubmitRef.current(finalContent, pubkeys, mediaTags);
    const collapseCompactComposer = () => {
      if (compact) setIsCompactExpanded(false);
    };

    // If onSubmit returns a promise, restore draft on failure.
    if (result && typeof result.then === "function") {
      result.then(collapseCompactComposer).catch(() => {
        setContent(savedContent);
        contentRef.current = savedContent;
        richText.setContent(savedContent);
        media.setPendingImeta(savedImeta);
        if (compact) setIsCompactExpanded(true);
      });
    } else {
      collapseCompactComposer();
    }
  }, [
    compact,
    media.pendingImetaRef,
    media.setPendingImeta,
    mentions.extractMentionPubkeys,
    mentions.clearMentions,
    channelLinks.clearChannels,
    richText.clearContent,
    richText.setContent,
  ]);
  submitMessageRef.current = submitMessage;

  const handleSubmit = React.useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      submitMessage();
    },
    [submitMessage],
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

      const { handled, suggestion } = mentions.handleMentionKeyDown(event);
      if (handled) {
        if (suggestion) {
          applyMentionInsert(suggestion);
        }
        return;
      }
    },
    [
      channelLinks.handleChannelKeyDown,
      applyChannelInsert,
      mentions.handleMentionKeyDown,
      applyMentionInsert,
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
          const mediaItem = items.find((item) =>
            ALLOWED_MEDIA_TYPES.includes(item.type),
          );
          if (mediaItem) {
            const file = mediaItem.getAsFile();
            if (file) {
              void uploadFileRef.current(file);
            }
            return true;
          }

          const html = event.clipboardData?.getData("text/html");
          if (html && hasMentionClipboardHtml(html)) {
            const cleanText = normalizeMentionClipboardHtml(html);
            event.preventDefault();
            _view.dispatch(
              _view.state.tr.insertText(
                cleanText,
                _view.state.selection.from,
                _view.state.selection.to,
              ),
            );
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
      (content.trim().length === 0 && media.pendingImeta.length === 0),
    [disabled, content, media.pendingImeta.length],
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
    <form
      className={cn(
        "relative rounded-2xl border border-input bg-card px-3 py-2 sm:px-4",
        className,
      )}
      onBlurCapture={handleFormBlur}
      onDragEnter={(event) => {
        expandCompactComposer();
        media.handleDragEnter(event);
      }}
      onDragLeave={media.handleDragLeave}
      onDragOver={media.handleDragOver}
      onDrop={(e) => {
        void media.handleDrop(e);
      }}
      onFocusCapture={expandCompactComposer}
      onSubmit={handleSubmit}
    >
      {media.isDragOver && <DropZoneOverlay />}
      {isCompactLayout ? (
        <ForumComposerCompactLayout
          editor={richText.editor}
          header={header}
          isSending={isSending}
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
              channelLinks.isChannelOpen ? channelLinks.channelSuggestions : []
            }
            mentionSelectedIndex={mentions.mentionSelectedIndex}
            mentionSuggestions={
              mentions.isMentionOpen ? mentions.suggestions : []
            }
            onChannelSelect={applyChannelInsert}
            onMentionSelect={applyMentionInsert}
            position={autocompletePosition}
          />

          <ForumComposerMediaStatus media={media} />

          {/* biome-ignore lint/a11y/noStaticElementInteractions: keydown handler bridges Tiptap editor to autocomplete and submit */}
          <div
            className="rich-text-composer max-h-32 overflow-y-auto"
            onKeyDown={handleEditorKeyDown}
          >
            <EditorContent editor={richText.editor} />
          </div>

          <MessageComposerToolbar
            composerDisabled={disabled ?? false}
            editor={richText.editor}
            extraActions={
              onCancel ? (
                <Button
                  disabled={isSending}
                  onClick={onCancel}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  Cancel
                </Button>
              ) : undefined
            }
            formattingDisabled={disabled ?? false}
            isEmojiPickerOpen={isEmojiPickerOpen}
            isFormattingOpen={isFormattingOpen}
            isSending={isSending ?? false}
            isUploading={media.isUploading}
            onCaptureSelection={handleToolbarMouseDown}
            onEmojiPickerOpenChange={setIsEmojiPickerOpen}
            onEmojiSelect={insertEmoji}
            onFormattingToggle={handleFormattingToggle}
            onOpenMentionPicker={openMentionPicker}
            onPaperclip={handlePaperclipClick}
            sendDisabled={sendDisabled}
          />
        </>
      )}
    </form>
  );
}
