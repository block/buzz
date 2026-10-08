import * as React from "react";
import type { Editor } from "@tiptap/react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { ALargeSmall, Mic, Paperclip, X } from "lucide-react";

import type { MediaUploadController } from "@/features/messages/lib/useMediaUpload";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
  type ComposerAddressAgent,
  ComposerMentionButton,
  ComposerSendButton,
} from "./ComposerAddressControls";
import { ComposerEmojiPicker } from "./ComposerEmojiPicker";
import { FormattingToolbar } from "./FormattingToolbar";
import { SelectionFormattingTray } from "./SelectionFormattingTray";

/** Spring for enter/exit of button groups — all fire simultaneously. */
const presenceSpring = {
  type: "spring",
  stiffness: 400,
  damping: 28,
} as const;
const ingressControlVariants = {
  exit: {
    opacity: 0,
    x: -12,
    transition: presenceSpring,
  },
} as const;
const NO_ADDRESSED_AGENTS: readonly ComposerAddressAgent[] = [];
const ignoreAddressRemoval = () => {};

export const MessageComposerToolbar = React.memo(
  function MessageComposerToolbar({
    addressedAgents = NO_ADDRESSED_AGENTS,
    autoPinConfirmationTitle,
    composerDisabled,
    editor,
    extraActions,
    formattingDisabled,
    gifMediaController,
    isEmojiPickerOpen,
    isFormattingOpen,
    isSending,
    isUploading,
    isVoiceNoteProcessing = false,
    isVoiceNoteRecording = false,
    hasVoiceNoteAttachment = false,
    voiceNoteRecorder,
    onCaptureSelection,
    onAutoPinConfirmationDismiss,
    onAutoPinConfirmationHoverChange,
    onAutoPinConfirmationTurnOff,
    onEmojiPickerOpenChange,
    onEmojiSelect,
    onFormattingToggle,
    onLinkButton,
    onOpenMentionPicker,
    onPaperclip,
    onFinishVoiceNote,
    onVoiceNote,
    onRemoveAddressedAgent = ignoreAddressRemoval,
    pulseVersionByPubkey,
    sendDisabled,
    shakeVersionByPubkey,
  }: {
    addressedAgents?: readonly ComposerAddressAgent[];
    autoPinConfirmationTitle?: string | null;
    composerDisabled: boolean;
    editor: Editor | null;
    extraActions?: React.ReactNode;
    formattingDisabled: boolean;
    gifMediaController: Pick<MediaUploadController, "setPendingImeta">;
    isEmojiPickerOpen: boolean;
    isFormattingOpen: boolean;
    isSending: boolean;
    isUploading: boolean;
    isVoiceNoteProcessing?: boolean;
    isVoiceNoteRecording?: boolean;
    hasVoiceNoteAttachment?: boolean;
    voiceNoteRecorder?: React.ReactNode;
    onCaptureSelection: () => void;
    onAutoPinConfirmationDismiss?: () => void;
    onAutoPinConfirmationHoverChange?: (hovered: boolean) => void;
    onAutoPinConfirmationTurnOff?: () => void;
    onEmojiPickerOpenChange: (open: boolean) => void;
    onEmojiSelect: (emoji: string) => void;
    onFormattingToggle: (pressed: boolean) => void;
    onLinkButton: () => void;
    onOpenMentionPicker: () => void;
    onPaperclip: () => void;
    onFinishVoiceNote?: () => void;
    onVoiceNote?: () => void;
    onRemoveAddressedAgent?: (pubkey: string) => void;
    pulseVersionByPubkey?: Readonly<Record<string, number>>;
    sendDisabled: boolean;
    shakeVersionByPubkey?: Readonly<Record<string, number>>;
  }) {
    const shouldReduceMotion = useReducedMotion();

    return (
      <div
        className="mt-2 flex flex-wrap items-center justify-between gap-3"
        data-testid="message-composer-toolbar"
      >
        <SelectionFormattingTray
          disabled={formattingDisabled}
          editor={editor}
          onLinkButton={onLinkButton}
        />
        <div className="-ml-2 flex min-h-10 min-w-0 flex-1 items-center gap-1 py-1">
          {/*
           * AnimatePresence with mode="popLayout" — exiting elements
           * are popped out of flow immediately so entering elements
           * can animate in simultaneously. No sequencing.
           *
           * The Aa toggle is duplicated inside both groups so
           * AnimatePresence handles the crossfade.
           */}
          <AnimatePresence mode="popLayout" initial={false}>
            {voiceNoteRecorder ? (
              <motion.div
                key="voice-note-controls"
                className="flex min-w-0 flex-1 items-center"
                data-testid="voice-note-controls"
                initial={shouldReduceMotion ? false : { opacity: 0, x: 12 }}
                animate={{ opacity: 1, x: 0 }}
                exit={
                  shouldReduceMotion ? { opacity: 0 } : { opacity: 0, x: 12 }
                }
                transition={
                  shouldReduceMotion ? { duration: 0 } : presenceSpring
                }
              >
                {voiceNoteRecorder}
              </motion.div>
            ) : isFormattingOpen ? (
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
                  <Tooltip disableHoverableContent>
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
                  <Tooltip disableHoverableContent>
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
                data-testid="composer-ingress-controls"
                initial={{ opacity: 0, x: -12 }}
                animate={{ opacity: 1, x: 0 }}
                exit="exit"
                variants={ingressControlVariants}
                transition={presenceSpring}
              >
                <ComposerMentionButton
                  agents={addressedAgents}
                  confirmationTitle={autoPinConfirmationTitle}
                  disabled={composerDisabled}
                  onConfirmationDismiss={onAutoPinConfirmationDismiss}
                  onConfirmationHoverChange={onAutoPinConfirmationHoverChange}
                  onConfirmationTurnOff={onAutoPinConfirmationTurnOff}
                  onCaptureSelection={onCaptureSelection}
                  onOpen={onOpenMentionPicker}
                  onRemove={onRemoveAddressedAgent}
                  pulseVersionByPubkey={pulseVersionByPubkey}
                  shakeVersionByPubkey={shakeVersionByPubkey}
                  showAgents
                />
                <Tooltip disableHoverableContent>
                  <TooltipTrigger asChild>
                    <Button
                      aria-label="Attach file"
                      disabled={
                        composerDisabled ||
                        isUploading ||
                        isVoiceNoteRecording ||
                        hasVoiceNoteAttachment
                      }
                      onClick={onPaperclip}
                      onMouseDown={onCaptureSelection}
                      size="icon"
                      type="button"
                      variant="ghost"
                    >
                      <Paperclip />
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>Attach file</TooltipContent>
                </Tooltip>
                {onVoiceNote ? (
                  <Tooltip disableHoverableContent>
                    <TooltipTrigger asChild>
                      <Button
                        aria-label="Record voice note"
                        disabled={composerDisabled || isUploading}
                        onClick={onVoiceNote}
                        onMouseDown={onCaptureSelection}
                        size="icon"
                        type="button"
                        variant="ghost"
                      >
                        <span className="inline-flex">
                          <Mic />
                        </span>
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>Record voice note</TooltipContent>
                  </Tooltip>
                ) : null}
                <ComposerEmojiPicker
                  disabled={composerDisabled || isVoiceNoteRecording}
                  gifsDisabled={hasVoiceNoteAttachment}
                  gifMediaController={gifMediaController}
                  onClose={() => editor?.commands.focus()}
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
                  <Tooltip disableHoverableContent>
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
          <ComposerSendButton
            isSending={isSending}
            onFinishVoiceNote={
              isVoiceNoteRecording ? onFinishVoiceNote : undefined
            }
            sendDisabled={
              isVoiceNoteRecording ? isVoiceNoteProcessing : sendDisabled
            }
          />
        </div>
      </div>
    );
  },
);
