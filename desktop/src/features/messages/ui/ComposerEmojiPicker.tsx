import { useQuery } from "@tanstack/react-query";
import { Images, Smile, SmilePlus } from "lucide-react";
import { motion, useReducedMotion } from "motion/react";
import * as React from "react";

import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import { klipyGifAttachment, type KlipyGif } from "@/features/gifs/api";
import { relayKlipyEndpoints, reportKlipyShare } from "@/features/gifs/relay";
import { KlipyGifPicker } from "@/features/gifs/ui/KlipyGifPicker";
import { useCommunities } from "@/features/communities/useCommunities";
import type { MediaUploadController } from "@/features/messages/lib/useMediaUpload";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type ComposerEmojiPickerProps = {
  disabled?: boolean;
  gifsDisabled?: boolean;
  gifMediaController: Pick<MediaUploadController, "setPendingImeta">;
  /** Called when the popover closes without an emoji selection (Escape,
   *  click-outside). Use this to restore focus to the editor. */
  onClose?: () => void;
  onEmojiSelect: (emoji: string) => void;
  onOpenChange: (open: boolean) => void;
  onTriggerMouseDown: () => void;
  open: boolean;
};

type ComposerPickerTab = "emoji" | "gifs";

const PICKER_TAB_ORDER: ComposerPickerTab[] = ["emoji", "gifs"];
const ACTIVE_TAB_OUTER_CORNER_RADIUS = "calc(var(--radius-2xl) - 0.375rem)";

export const ComposerEmojiPicker = React.memo(function ComposerEmojiPicker({
  disabled = false,
  gifsDisabled = false,
  gifMediaController,
  onClose,
  onEmojiSelect,
  onOpenChange,
  onTriggerMouseDown,
  open,
}: ComposerEmojiPickerProps) {
  const [pickerTab, setPickerTab] = React.useState<ComposerPickerTab>("emoji");
  const shouldReduceMotion = useReducedMotion();
  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl ?? "";
  const klipyCapabilityQuery = useQuery({
    enabled: relayUrl.length > 0,
    queryFn: ({ signal }) => relayKlipyEndpoints(relayUrl, signal),
    queryKey: ["relay-capability", relayUrl, "klipy"],
    retry: 1,
    staleTime: 5 * 60 * 1_000,
  });
  const gifSearchPath = klipyCapabilityQuery.data?.searchPath ?? "";
  const gifSharePath = klipyCapabilityQuery.data?.sharePath ?? "";
  const gifsAvailable = gifSearchPath.length > 0 && gifSharePath.length > 0;

  React.useEffect(() => {
    if (!open) setPickerTab("emoji");
  }, [open]);

  const handleGifSelect = React.useCallback(
    (gif: KlipyGif) => {
      if (!relayUrl || gifsDisabled) return;
      onOpenChange(false);
      gifMediaController.setPendingImeta((current) => [
        ...current,
        klipyGifAttachment(gif),
      ]);
      void reportKlipyShare(relayUrl, gifSharePath, gif.slug).catch(() => {
        // Sharing the GIF remains successful if best-effort Recents reporting
        // fails; the authenticated relay endpoint keeps provider details hidden.
      });
    },
    [
      gifMediaController.setPendingImeta,
      gifsDisabled,
      gifSharePath,
      onOpenChange,
      relayUrl,
    ],
  );

  return (
    <Popover onOpenChange={onOpenChange} open={open}>
      <Tooltip disableHoverableContent>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              aria-label={
                gifsAvailable ? "Insert emoji or GIF" : "Insert emoji"
              }
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
        <TooltipContent>
          {gifsAvailable ? "Emoji and GIFs" : "Emoji"}
        </TooltipContent>
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
        {gifsAvailable ? (
          <Tabs
            className="w-[352px] overflow-hidden rounded-2xl bg-muted"
            onValueChange={(value) => setPickerTab(value as ComposerPickerTab)}
            value={pickerTab}
          >
            <TabsList className="relative isolate grid h-11 w-full grid-cols-2 overflow-hidden rounded-none border-b border-border/60 bg-muted p-1.5 text-muted-foreground">
              <motion.div
                animate={{
                  transform: `translateX(${PICKER_TAB_ORDER.indexOf(pickerTab) * 100}%)`,
                }}
                aria-hidden="true"
                className="absolute bottom-1.5 left-1.5 top-1.5 z-0 rounded-md bg-background shadow"
                initial={false}
                style={{
                  borderTopLeftRadius:
                    pickerTab === "emoji"
                      ? ACTIVE_TAB_OUTER_CORNER_RADIUS
                      : undefined,
                  borderTopRightRadius:
                    pickerTab === "gifs"
                      ? ACTIVE_TAB_OUTER_CORNER_RADIUS
                      : undefined,
                  width: "calc((100% - 12px) / 2)",
                }}
                transition={
                  shouldReduceMotion
                    ? { duration: 0 }
                    : {
                        duration: 0.18,
                        ease: [0.77, 0, 0.175, 1],
                      }
                }
              />
              <TabsTrigger
                className="relative z-10 h-8 gap-1.5 bg-transparent shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:shadow-none"
                value="emoji"
              >
                <Smile aria-hidden className="h-4 w-4" />
                Emoji
              </TabsTrigger>
              <TabsTrigger
                className="relative z-10 h-8 gap-1.5 bg-transparent shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:shadow-none"
                disabled={gifsDisabled}
                value="gifs"
              >
                <Images aria-hidden className="h-4 w-4" />
                GIFs
              </TabsTrigger>
            </TabsList>
            <TabsContent className="m-0" value="emoji">
              <EmojiPicker autoFocus onSelect={onEmojiSelect} perLine={9} />
            </TabsContent>
            <TabsContent className="m-0" value="gifs">
              <KlipyGifPicker
                onSelect={handleGifSelect}
                relayUrl={relayUrl}
                searchPath={gifSearchPath}
              />
            </TabsContent>
          </Tabs>
        ) : (
          <EmojiPicker autoFocus onSelect={onEmojiSelect} perLine={9} />
        )}
      </PopoverContent>
    </Popover>
  );
});
