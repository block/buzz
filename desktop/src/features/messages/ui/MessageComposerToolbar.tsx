import * as React from "react";
import type { Editor } from "@tiptap/react";
import { AnimatePresence, motion } from "motion/react";
import {
  ALargeSmall,
  ArrowUp,
  AtSign,
  HatGlasses,
  Paperclip,
  X,
} from "lucide-react";

import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { ComposerEmojiPicker } from "./ComposerEmojiPicker";
import {
  FormattingToolbar,
  isSpoilerFormattingActive,
  type SpoilerToggleState,
  toggleSpoilerFormatting,
} from "./FormattingToolbar";

/** Spring for enter/exit of button groups — all fire simultaneously. */
const presenceSpring = {
  type: "spring",
  stiffness: 400,
  damping: 28,
} as const;

export const MessageComposerToolbar = React.memo(
  function MessageComposerToolbar({
    composerDisabled,
    editor,
    extraActions,
    formattingDisabled,
    isEmojiPickerOpen,
    isFormattingOpen,
    isSending,
    isUploading,
    onCaptureSelection,
    onEmojiPickerOpenChange,
    onEmojiSelect,
    onFormattingToggle,
    onLinkButton,
    onOpenMentionPicker,
    onPaperclip,
    onSpoilerToggle,
    sendDisabled,
    spoilerActive,
  }: {
    composerDisabled: boolean;
    editor: Editor | null;
    extraActions?: React.ReactNode;
    formattingDisabled: boolean;
    isEmojiPickerOpen: boolean;
    isFormattingOpen: boolean;
    isSending: boolean;
    isUploading: boolean;
    onCaptureSelection: () => void;
    onEmojiPickerOpenChange: (open: boolean) => void;
    onEmojiSelect: (emoji: string) => void;
    onFormattingToggle: (pressed: boolean) => void;
    onLinkButton: () => void;
    onOpenMentionPicker: () => void;
    onPaperclip: () => void;
    onSpoilerToggle?: (state: SpoilerToggleState) => void;
    sendDisabled: boolean;
    spoilerActive?: boolean;
  }) {
    const [spoilerFormattingActive, setSpoilerFormattingActive] =
      React.useState(() =>
        editor ? isSpoilerFormattingActive(editor) : false,
      );

    React.useEffect(() => {
      if (!editor) {
        setSpoilerFormattingActive(false);
        return;
      }

      const update = () => {
        setSpoilerFormattingActive(isSpoilerFormattingActive(editor));
      };
      update();
      editor.on("transaction", update);
      return () => {
        editor.off("transaction", update);
      };
    }, [editor]);

    const isSpoilerActive = spoilerFormattingActive || Boolean(spoilerActive);

    const handleSpoilerClick = React.useCallback(() => {
      if (!editor) return;
      onSpoilerToggle?.(toggleSpoilerFormatting(editor));
    }, [editor, onSpoilerToggle]);

    return (
      <div className="mt-2 flex flex-wrap items-center justify-between gap-3">
        <div className="flex min-h-10 min-w-0 flex-1 items-center gap-1 py-1">
          {/*
           * AnimatePresence with mode="popLayout" — exiting elements
           * are popped out of flow immediately so entering elements
           * can animate in simultaneously. No sequencing.
           *
           * The Aa toggle is duplicated inside both groups so
           * AnimatePresence handles the crossfade. No layoutId,
           * no order hacks, no overflow clipping needed.
           */}
          <AnimatePresence mode="popLayout" initial={false}>
            {isFormattingOpen ? (
              /*
               * ── Expanded: [Aa] [✕] | [formatting buttons] ──
               */
              <motion.div
                key="formatting-controls"
                className="flex min-w-0 flex-1 items-center gap-1"
                initial={false}
                animate={{}}
                exit={{ opacity: 0 }}
                transition={presenceSpring}
              >
                <motion.div
                  initial={{ x: 8, opacity: 0 }}
                  animate={{ x: 0, opacity: 1 }}
                  exit={{ x: 8, opacity: 0 }}
                  transition={presenceSpring}
                >
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Button
                        aria-label="Toggle formatting"
                        aria-pressed={isFormattingOpen}
                        disabled={composerDisabled}
                        onClick={() => onFormattingToggle(!isFormattingOpen)}
                        onMouseDown={onCaptureSelection}
                        size="icon"
                        type="button"
                        variant={isFormattingOpen ? "default" : "ghost"}
                      >
                        <ALargeSmall />
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>Formatting</TooltipContent>
                  </Tooltip>
                </motion.div>
                <motion.div
                  className="flex items-center gap-1"
                  initial={{ opacity: 0, scale: 0.95 }}
                  animate={{ opacity: 1, scale: 1 }}
                  exit={{ opacity: 0, scale: 0.95 }}
                  transition={{ ...presenceSpring, delay: 0.15 }}
                >
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Button
                        aria-label="Close formatting"
                        disabled={composerDisabled}
                        onClick={() => onFormattingToggle(false)}
                        onMouseDown={onCaptureSelection}
                        size="icon"
                        type="button"
                        variant="ghost"
                        className="shrink-0"
                      >
                        <X />
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>Close formatting</TooltipContent>
                  </Tooltip>
                  <div className="mx-1 h-5 w-px shrink-0 bg-border/60" />
                </motion.div>
                <motion.div
                  className="min-w-0 flex-1 overflow-x-auto"
                  initial={{ opacity: 0, scale: 0.95 }}
                  animate={{ opacity: 1, scale: 1 }}
                  exit={{ opacity: 0, scale: 0.95 }}
                  transition={{ ...presenceSpring, delay: 0.15 }}
                >
                  <FormattingToolbar
                    editor={editor}
                    disabled={formattingDisabled}
                    onLinkButton={onLinkButton}
                  />
                </motion.div>
              </motion.div>
            ) : (
              /*
               * ── Passive: [@ 📎 😊] [Aa] ──
               */
              <motion.div
                key="ingress-controls"
                className="flex items-center gap-1"
                initial={{ opacity: 0, x: -12 }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: -12 }}
                transition={presenceSpring}
              >
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      aria-label="Mention someone"
                      data-testid="message-insert-mention"
                      disabled={composerDisabled}
                      onClick={onOpenMentionPicker}
                      onMouseDown={onCaptureSelection}
                      size="icon"
                      type="button"
                      variant="ghost"
                    >
                      <AtSign />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Mention someone</TooltipContent>
                </Tooltip>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      aria-label="Attach image"
                      disabled={composerDisabled || isUploading}
                      onClick={onPaperclip}
                      onMouseDown={onCaptureSelection}
                      size="icon"
                      type="button"
                      variant="ghost"
                    >
                      <Paperclip />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Attach image</TooltipContent>
                </Tooltip>
                <ComposerEmojiPicker
                  disabled={composerDisabled}
                  onEmojiSelect={onEmojiSelect}
                  onOpenChange={onEmojiPickerOpenChange}
                  onTriggerMouseDown={onCaptureSelection}
                  open={isEmojiPickerOpen}
                />
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      aria-label="Spoiler"
                      aria-pressed={isSpoilerActive}
                      className={cn(
                        isSpoilerActive &&
                          "bg-primary text-primary-foreground hover:bg-primary/90 hover:text-primary-foreground",
                      )}
                      disabled={composerDisabled || !editor || isUploading}
                      onClick={handleSpoilerClick}
                      onMouseDown={onCaptureSelection}
                      size="icon"
                      type="button"
                      variant={isSpoilerActive ? "default" : "ghost"}
                    >
                      <HatGlasses />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Spoiler</TooltipContent>
                </Tooltip>
                <motion.div
                  initial={{ x: -8, opacity: 0 }}
                  animate={{ x: 0, opacity: 1 }}
                  exit={{ x: -8, opacity: 0 }}
                  transition={presenceSpring}
                >
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Button
                        aria-label="Toggle formatting"
                        aria-pressed={isFormattingOpen}
                        disabled={composerDisabled}
                        onClick={() => onFormattingToggle(!isFormattingOpen)}
                        onMouseDown={onCaptureSelection}
                        size="icon"
                        type="button"
                        variant={isFormattingOpen ? "default" : "ghost"}
                      >
                        <ALargeSmall />
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>Formatting</TooltipContent>
                  </Tooltip>
                </motion.div>
              </motion.div>
            )}
          </AnimatePresence>
        </div>

        <div className="flex items-center gap-2">
          {extraActions}
          <Button
            aria-label={isSending ? "Sending" : "Send message"}
            className="rounded-full"
            data-testid="send-message"
            disabled={sendDisabled || isSending}
            size="icon"
            type="submit"
          >
            {isSending ? (
              <span
                aria-hidden
                className="h-4 w-4 animate-spin rounded-full border-2 border-primary-foreground border-t-transparent"
              />
            ) : (
              <ArrowUp aria-hidden />
            )}
          </Button>
        </div>
      </div>
    );
  },
);
