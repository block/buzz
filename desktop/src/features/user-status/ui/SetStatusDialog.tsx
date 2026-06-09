import * as React from "react";
import * as PopoverPrimitive from "@radix-ui/react-popover";

import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverTrigger } from "@/shared/ui/popover";

const PRESETS = [
  { text: "In a meeting", emoji: "\uD83D\uDDE3\uFE0F" },
  { text: "Commuting", emoji: "\uD83D\uDE8C" },
  { text: "Out sick", emoji: "\uD83E\uDD12" },
  { text: "Vacationing", emoji: "\uD83C\uDFD6\uFE0F" },
  { text: "Working remotely", emoji: "\uD83C\uDFE0" },
] as const;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type SetStatusDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialText?: string;
  initialEmoji?: string;
  onSave: (text: string, emoji: string) => void;
  onClear: () => void;
  hasExistingStatus: boolean;
};

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function SetStatusDialog({
  open,
  onOpenChange,
  initialText = "",
  initialEmoji = "",
  onSave,
  onClear,
  hasExistingStatus,
}: SetStatusDialogProps) {
  const [text, setText] = React.useState(initialText);
  const [emoji, setEmoji] = React.useState(initialEmoji);
  const [pickerOpen, setPickerOpen] = React.useState(false);

  React.useEffect(() => {
    if (open) {
      setText(initialText);
      setEmoji(initialEmoji);
    }
  }, [open, initialText, initialEmoji]);

  function handlePresetClick(preset: { text: string; emoji: string }) {
    setText(preset.text);
    setEmoji(preset.emoji);
  }

  function handleEmojiSelect(selectedEmoji: string) {
    setEmoji(selectedEmoji);
    setPickerOpen(false);
  }

  function handleSave() {
    onSave(text.trim(), emoji);
    onOpenChange(false);
  }

  function handleClear() {
    onClear();
    onOpenChange(false);
  }

  function handleKeyDown(event: React.KeyboardEvent) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      handleSave();
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="sm:max-w-[420px]"
        data-testid="set-status-dialog"
      >
        <DialogHeader>
          <DialogTitle>Set a status</DialogTitle>
          <DialogDescription>
            Let others know what you're up to.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4 pt-2">
          <div className="flex items-center gap-2">
            <Popover onOpenChange={setPickerOpen} open={pickerOpen}>
              <div className="relative shrink-0">
                <PopoverTrigger asChild>
                  <button
                    aria-label="Choose status emoji"
                    className="flex h-9 w-9 items-center justify-center rounded-md border border-input text-lg transition-colors hover:bg-accent"
                    type="button"
                  >
                    {emoji ? (
                      <StatusEmoji className="h-5 w-5" value={emoji} />
                    ) : (
                      "\uD83D\uDCAC"
                    )}
                  </button>
                </PopoverTrigger>
                {emoji ? (
                  <button
                    aria-label="Clear status emoji"
                    className="absolute -right-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full border border-background bg-muted text-[10px] leading-none text-muted-foreground hover:bg-accent hover:text-foreground"
                    onClick={(event) => {
                      event.stopPropagation();
                      setEmoji("");
                    }}
                    type="button"
                  >
                    ×
                  </button>
                ) : null}
              </div>
              <PopoverPrimitive.Content
                align="start"
                sideOffset={4}
                className="z-50 w-auto overflow-hidden rounded-2xl shadow-md outline-hidden data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95"
              >
                <EmojiPicker autoFocus onSelect={handleEmojiSelect} />
              </PopoverPrimitive.Content>
            </Popover>
            <Input
              autoFocus
              data-testid="set-status-input"
              onChange={(event) => setText(event.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="What's your status?"
              value={text}
            />
          </div>

          <div className="flex flex-wrap gap-1.5">
            {PRESETS.map((preset) => (
              <button
                className="rounded-full border border-input px-2.5 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                data-testid={`set-status-preset-${preset.text.toLowerCase().replace(/\s+/g, "-")}`}
                key={preset.text}
                onClick={() => handlePresetClick(preset)}
                type="button"
              >
                {preset.emoji} {preset.text}
              </button>
            ))}
          </div>

          <div className="flex items-center justify-between gap-2 pt-1">
            <div>
              {hasExistingStatus ? (
                <Button
                  data-testid="set-status-clear"
                  onClick={handleClear}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  Clear status
                </Button>
              ) : null}
            </div>
            <div className="flex items-center gap-2">
              <Button
                data-testid="set-status-cancel"
                onClick={() => onOpenChange(false)}
                size="sm"
                type="button"
                variant="ghost"
              >
                Cancel
              </Button>
              <Button
                data-testid="set-status-save"
                disabled={!text.trim() && !emoji}
                onClick={handleSave}
                size="sm"
                type="button"
              >
                Save
              </Button>
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
