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
import { ChannelAutocomplete } from "@/features/messages/ui/ChannelAutocomplete";
import { ComposerAttachments } from "@/features/messages/ui/ComposerAttachments";
import {
  MentionAutocomplete,
  type MentionSuggestion,
} from "@/features/messages/ui/MentionAutocomplete";
import { MessageComposerToolbar } from "@/features/messages/ui/MessageComposerToolbar";
import type { ChannelMember } from "@/shared/api/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { Button } from "@/shared/ui/button";

type ForumComposerProps = {
  channelId?: string | null;
  /** Override mention source when no channel is available (e.g. Pulse). */
  members?: ChannelMember[];
  placeholder: string;
  disabled?: boolean;
  isSending?: boolean;
  onCancel?: () => void;
  onSubmit: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => undefined | Promise<unknown>;
  /** When true, autocomplete renders below the input (for top-of-view composers). */
  autocompleteBelow?: boolean;
  profiles?: UserProfileLookup;
};

export function ForumComposer({
  channelId = null,
  members,
  placeholder,
  disabled,
  isSending,
  onCancel,
  onSubmit,
  autocompleteBelow = false,
  profiles,
}: ForumComposerProps) {
  const [content, setContent] = React.useState("");
  const contentRef = React.useRef(content);
  contentRef.current = content;

  const [isEmojiPickerOpen, setIsEmojiPickerOpen] = React.useState(false);
  const [isFormattingOpen, setIsFormattingOpen] = React.useState(false);

  const handleFormattingToggle = React.useCallback((pressed: boolean) => {
    if (pressed) setIsEmojiPickerOpen(false);
    setIsFormattingOpen(pressed);
  }, []);

  const mentions = useMentions(channelId, members, profiles);
  const channelLinks = useChannelLinks();
  const media = useMediaUpload();

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

      const { cursor } = richText.getTextAndCursor();
      mentions.updateMentionQuery(text, cursor);
      channelLinks.updateChannelQuery(text, cursor);
    },
  });

  // ── Mention / channel autocomplete insertion ────────────────────────
  const applyMentionInsert = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const { text, cursor } = richText.getTextAndCursor();
      const result = mentions.insertMention(suggestion, text, cursor);
      richText.setContentWithTrailingSpace(result.nextContent);
      setContent(result.nextContent);
      contentRef.current = result.nextContent;
    },
    [
      mentions.insertMention,
      richText.getTextAndCursor,
      richText.setContentWithTrailingSpace,
    ],
  );

  const applyChannelInsert = React.useCallback(
    (suggestion: ChannelSuggestion) => {
      const { text, cursor } = richText.getTextAndCursor();
      const result = channelLinks.insertChannel(suggestion, text, cursor);
      richText.setContentWithTrailingSpace(result.nextContent);
      setContent(result.nextContent);
      contentRef.current = result.nextContent;
    },
    [
      channelLinks.insertChannel,
      richText.getTextAndCursor,
      richText.setContentWithTrailingSpace,
    ],
  );

  // ── Emoji insertion ─────────────────────────────────────────────────
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
    const { text, cursor } = richText.getTextAndCursor();

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

    const updatedText = richText.editor.state.doc.textContent;
    const { cursor: updatedCursor } = richText.getTextAndCursor();
    mentions.updateMentionQuery(updatedText, updatedCursor);
  }, [
    richText.editor,
    richText.getTextAndCursor,
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

    // If onSubmit returns a promise, restore draft on failure.
    if (result && typeof result.then === "function") {
      result.catch(() => {
        setContent(savedContent);
        contentRef.current = savedContent;
        richText.setContent(savedContent);
        media.setPendingImeta(savedImeta);
      });
    }
  }, [
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

  // ── Send button state ───────────────────────────────────────────────
  const sendDisabled = React.useMemo(
    () =>
      disabled ||
      (content.trim().length === 0 && media.pendingImeta.length === 0),
    [disabled, content, media.pendingImeta.length],
  );

  const handlePaperclipClick = React.useCallback(() => {
    void media.handlePaperclip();
  }, [media.handlePaperclip]);

  // ── Render ──────────────────────────────────────────────────────────
  const autocompletePosition = autocompleteBelow ? "below" : "above";

  return (
    <form
      className="relative rounded-2xl border border-input bg-card px-3 py-2 sm:px-4"
      onDragOver={media.handleDragOver}
      onDrop={(e) => {
        void media.handleDrop(e);
      }}
      onSubmit={handleSubmit}
    >
      <ChannelAutocomplete
        onSelect={applyChannelInsert}
        position={autocompletePosition}
        selectedIndex={channelLinks.channelSelectedIndex}
        suggestions={
          channelLinks.isChannelOpen ? channelLinks.channelSuggestions : []
        }
      />
      <MentionAutocomplete
        onSelect={applyMentionInsert}
        position={autocompletePosition}
        selectedIndex={mentions.mentionSelectedIndex}
        suggestions={mentions.isMentionOpen ? mentions.suggestions : []}
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

      {(media.pendingImeta.length > 0 || media.isUploading) && (
        <div className="mb-2 flex items-center gap-2">
          <ComposerAttachments
            attachments={media.pendingImeta}
            isUploading={media.isUploading}
            uploadingCount={media.uploadingCount}
            onRemove={media.removeAttachment}
          />
        </div>
      )}

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
        onCaptureSelection={() => {}}
        onEmojiPickerOpenChange={setIsEmojiPickerOpen}
        onEmojiSelect={insertEmoji}
        onFormattingToggle={handleFormattingToggle}
        onOpenMentionPicker={openMentionPicker}
        onPaperclip={handlePaperclipClick}
        sendDisabled={sendDisabled}
      />
    </form>
  );
}
