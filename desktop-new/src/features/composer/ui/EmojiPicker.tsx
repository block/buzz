import { Popover as BasePopover } from "@base-ui/react/popover";
import { IconMoodSmile } from "@tabler/icons-react";
import { useState, type RefObject } from "react";

import { Button } from "@/shared/ui/Button";
import { IconButton } from "@/shared/ui/IconButton";

const EMOJI = [
  "😀",
  "😂",
  "😊",
  "😍",
  "🤔",
  "🎉",
  "👍",
  "👀",
  "🙏",
  "💜",
  "🔥",
  "✅",
  "🚀",
  "💡",
  "🫡",
  "👋",
  "🙌",
  "💯",
  "🤝",
  "✨",
] as const;

/**
 * The first Composer expression control: a small, curated emoji picker.
 *
 * Base UI owns the anchored, portal-rendered popup and its dismissal. The
 * composer owns selection restoration: picker interactions must not turn a
 * chosen emoji into an editor-focus detour.
 */
export function EmojiPicker({
  disabled,
  textarea,
  onInsert,
}: {
  disabled?: boolean;
  textarea: RefObject<HTMLTextAreaElement | null>;
  onInsert: (emoji: string) => void;
}) {
  const [open, setOpen] = useState(false);

  return (
    <BasePopover.Root modal={false} open={open} onOpenChange={setOpen}>
      <BasePopover.Trigger
        render={
          <IconButton
            aria-label="Choose emoji"
            disabled={disabled}
            icon={<IconMoodSmile size={16} stroke={2} aria-hidden="true" />}
            size="toolbar"
            variant="ghost"
            onMouseDown={(event) => event.preventDefault()}
          />
        }
      />
      <BasePopover.Portal>
        <BasePopover.Positioner side="top" align="start" sideOffset={8}>
          <BasePopover.Popup
            className="composer-emoji-picker"
            initialFocus={false}
            finalFocus={textarea}
          >
            <fieldset className="composer-emoji-grid">
              <legend className="sr-only">Emoji</legend>
              {EMOJI.map((emoji) => (
                <Button
                  key={emoji}
                  aria-label={`Insert ${emoji}`}
                  size="compact"
                  variant="ghost"
                  onClick={() => {
                    onInsert(emoji);
                    setOpen(false);
                  }}
                >
                  <span aria-hidden="true">{emoji}</span>
                </Button>
              ))}
            </fieldset>
          </BasePopover.Popup>
        </BasePopover.Positioner>
      </BasePopover.Portal>
    </BasePopover.Root>
  );
}
