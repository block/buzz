import { Images, Smile, SmilePlus } from "lucide-react";
import * as React from "react";

import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import {
  downloadKlipyGif,
  type KlipyGif,
  trackKlipyGifShare,
} from "@/features/gifs/api";
import { KlipyGifPicker } from "@/features/gifs/ui/KlipyGifPicker";
import type { MediaUploadController } from "@/features/messages/lib/useMediaUpload";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type ComposerEmojiPickerProps = {
  disabled?: boolean;
  gifUploadController: Pick<
    MediaUploadController,
    "setUploadState" | "uploadFile"
  >;
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
  gifUploadController,
  onClose,
  onEmojiSelect,
  onOpenChange,
  onTriggerMouseDown,
  open,
}: ComposerEmojiPickerProps) {
  const handleGifSelect = React.useCallback(
    (gif: KlipyGif) => {
      onOpenChange(false);
      void downloadKlipyGif(gif)
        .then(gifUploadController.uploadFile)
        .catch((error: unknown) => {
          gifUploadController.setUploadState({
            status: "error",
            message: String(error),
          });
        });
      // Provider analytics must never block the user's attachment flow.
      void trackKlipyGifShare(gif.slug).catch(() => undefined);
    },
    [
      gifUploadController.setUploadState,
      gifUploadController.uploadFile,
      onOpenChange,
    ],
  );

  return (
    <Popover onOpenChange={onOpenChange} open={open}>
      <Tooltip disableHoverableContent>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              aria-label="Insert emoji or GIF"
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
        <TooltipContent>Emoji and GIFs</TooltipContent>
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
        <Tabs
          className="w-[352px] overflow-hidden rounded-2xl bg-popover"
          defaultValue="emoji"
        >
          <TabsList className="h-11 w-full rounded-none border-b border-border/60 bg-popover p-1.5">
            <TabsTrigger className="h-8 flex-1 gap-1.5" value="emoji">
              <Smile aria-hidden className="h-4 w-4" />
              Emoji
            </TabsTrigger>
            <TabsTrigger className="h-8 flex-1 gap-1.5" value="gifs">
              <Images aria-hidden className="h-4 w-4" />
              GIFs
            </TabsTrigger>
          </TabsList>
          <TabsContent className="m-0" value="emoji">
            <EmojiPicker autoFocus onSelect={onEmojiSelect} perLine={9} />
          </TabsContent>
          <TabsContent className="m-0" value="gifs">
            <KlipyGifPicker onSelect={handleGifSelect} />
          </TabsContent>
        </Tabs>
      </PopoverContent>
    </Popover>
  );
});
