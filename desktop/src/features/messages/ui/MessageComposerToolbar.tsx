import * as React from "react";
import type { Editor } from "@tiptap/react";
import { AnimatePresence, motion } from "motion/react";
import { ALargeSmall, ArrowUp, AtSign, Paperclip, X } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { ComposerEmojiPicker } from "./ComposerEmojiPicker";
import { FormattingToolbar } from "./FormattingToolbar";

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
    onOpenMentionPicker,
    onPaperclip,
    sendDisabled,
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
    onOpenMentionPicker: () => void;
    onPaperclip: () => void;
    sendDisabled: boolean;
  }) {
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
                      <button
                        type="button"
                        aria-label="Toggle formatting"
                        aria-pressed={isFormattingOpen}
                        disabled={composerDisabled}
                        onClick={() => onFormattingToggle(!isFormattingOpen)}
                        onMouseDown={onCaptureSelection}
                        className={cn(
                          "inline-flex h-8 min-w-8 items-center justify-center rounded-md px-2 text-sm font-medium transition-colors",
                          "hover:bg-muted hover:text-foreground",
                          "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
                          "disabled:pointer-events-none disabled:opacity-50",
                          "[&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
                          isFormattingOpen
                            ? "bg-primary text-primary-foreground"
                            : "bg-transparent text-muted-foreground",
                        )}
                      >
                        <ALargeSmall className="h-4 w-4" />
                      </button>
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
                        className="h-7 w-7 shrink-0"
                      >
                        <X className="h-3.5 w-3.5" />
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
                      <AtSign className="h-4 w-4" />
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
                      {isUploading ? (
                        <Spinner className="h-4 w-4" />
                      ) : (
                        <Paperclip className="h-4 w-4" />
                      )}
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
                <motion.div
                  initial={{ x: -8, opacity: 0 }}
                  animate={{ x: 0, opacity: 1 }}
                  exit={{ x: -8, opacity: 0 }}
                  transition={presenceSpring}
                >
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <button
                        type="button"
                        aria-label="Toggle formatting"
                        aria-pressed={isFormattingOpen}
                        disabled={composerDisabled}
                        onClick={() => onFormattingToggle(!isFormattingOpen)}
                        onMouseDown={onCaptureSelection}
                        className={cn(
                          "inline-flex h-8 min-w-8 items-center justify-center rounded-md px-2 text-sm font-medium transition-colors",
                          "hover:bg-muted hover:text-foreground",
                          "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
                          "disabled:pointer-events-none disabled:opacity-50",
                          "[&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
                          isFormattingOpen
                            ? "bg-primary text-primary-foreground"
                            : "bg-transparent text-muted-foreground",
                        )}
                      >
                        <ALargeSmall className="h-4 w-4" />
                      </button>
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
              <ArrowUp aria-hidden className="h-4 w-4" />
            )}
          </Button>
        </div>
      </div>
    );
  },
);
