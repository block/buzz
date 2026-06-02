import * as React from "react";
import Picker from "@emoji-mart/react";
import data from "@emoji-mart/data";
import { SmilePlus } from "lucide-react";

import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import { buildCustomEmojiCategory } from "@/features/custom-emoji/emojiMartCategory";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type ComposerEmojiPickerProps = {
  customEmoji?: CustomEmoji[];
  disabled?: boolean;
  onEmojiSelect: (emoji: string) => void;
  onOpenChange: (open: boolean) => void;
  onTriggerMouseDown: () => void;
  open: boolean;
};

export const ComposerEmojiPicker = React.memo(function ComposerEmojiPicker({
  customEmoji = [],
  disabled = false,
  onEmojiSelect,
  onOpenChange,
  onTriggerMouseDown,
  open,
}: ComposerEmojiPickerProps) {
  const custom = React.useMemo(
    () => buildCustomEmojiCategory(customEmoji),
    [customEmoji],
  );
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
              <SmilePlus className="h-4 w-4" />
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>Insert emoji</TooltipContent>
      </Tooltip>
      <PopoverContent
        align="start"
        className="w-auto p-0 rounded-2xl overflow-hidden border-0 bg-transparent shadow-none"
        side="top"
        sideOffset={10}
      >
        <Picker
          data={data}
          custom={custom}
          onEmojiSelect={(emoji: { native?: string; id?: string }) => {
            // Custom emoji have no `native`; insert their `:shortcode:` (the
            // emoji-mart id is the shortcode). Standard emoji insert `native`.
            if (emoji.native) {
              onEmojiSelect(emoji.native);
            } else if (emoji.id) {
              onEmojiSelect(`:${emoji.id}:`);
            }
          }}
          theme="auto"
          previewPosition="none"
          skinTonePosition="search"
          set="native"
          maxFrequentRows={2}
          perLine={8}
        />
      </PopoverContent>
    </Popover>
  );
});
