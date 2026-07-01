import { SmilePlus } from "lucide-react";
import * as React from "react";

import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type ComposerEmojiPickerProps = {
  disabled?: boolean;
  onEmojiSelect: (emoji: string) => void;
  onOpenChange: (open: boolean) => void;
  onTriggerMouseDown: () => void;
  open: boolean;
};

export const ComposerEmojiPicker = React.memo(function ComposerEmojiPicker({
  disabled = false,
  onEmojiSelect,
  onOpenChange,
  onTriggerMouseDown,
  open,
}: ComposerEmojiPickerProps) {
  return (
    <Popover onOpenChange={onOpenChange} open={open}>
      <Tooltip>
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
        // Prevent Radix from returning focus to the trigger button on
        // close — insertEmoji calls editor.chain().focus() before
        // setIsEmojiPickerOpen(false), so the editor owns focus by the
        // time the popover closes; let that stand.
        onCloseAutoFocus={(e) => e.preventDefault()}
        side="top"
        sideOffset={10}
      >
        <EmojiPicker autoFocus onSelect={onEmojiSelect} />
      </PopoverContent>
    </Popover>
  );
});
