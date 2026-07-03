import { SmilePlus } from "lucide-react";
import * as React from "react";

import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type ComposerEmojiPickerProps = {
  disabled?: boolean;
  /** Called when the popover closes without an emoji selection (Escape,
   *  click-outside). Use this to restore focus to the editor. */
  onClose?: () => void;
  onEmojiSelect: (emoji: string) => void;
  onOpenChange: (open: boolean) => void;
  onTriggerMouseDown: () => void;
  open: boolean;
};

export const ComposerEmojiPicker = React.memo(function ComposerEmojiPicker({
  disabled = false,
  onClose,
  onEmojiSelect,
  onOpenChange,
  onTriggerMouseDown,
  open,
}: ComposerEmojiPickerProps) {
  return (
    <Popover onOpenChange={onOpenChange} open={open}>
      <Tooltip disableHoverableContent>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              aria-label="Insert emoji"
              data-testid="composer-emoji-button"
              disabled={disabled}
              onMouseDown={onTriggerMouseDown}
              size="icon"
              type="button"
              variant="ghost"
            >
              <SmilePlus />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>Insert emoji</TooltipContent>
      </Tooltip>
      <PopoverContent
        align="start"
        className="w-auto p-0 rounded-2xl overflow-hidden border-0 bg-transparent shadow-none"
        // Prevent Radix's FocusScope from stealing focus on open — our
        // disableSearchInputCorrections MutationObserver owns focus for
        // the shadow-DOM search input (autoFocus path).
        onOpenAutoFocus={(e) => e.preventDefault()}
        // Suppress Radix's default trigger-return on close. On the
        // emoji-select path, insertEmoji already called editor.chain().focus()
        // before the popover closes, so the editor owns focus — let it stand.
        // On Escape/click-outside, onClose() restores editor focus explicitly
        // so the user can keep typing without an extra click.
        onCloseAutoFocus={(e) => {
          e.preventDefault();
          onClose?.();
        }}
        side="top"
        sideOffset={10}
      >
        <EmojiPicker autoFocus onSelect={onEmojiSelect} />
      </PopoverContent>
    </Popover>
  );
});
