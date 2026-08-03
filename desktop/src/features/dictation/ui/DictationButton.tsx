import { Mic, Square } from "lucide-react";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { cn } from "@/shared/lib/cn";
import { isMacPlatform } from "@/shared/lib/platform";

interface DictationState {
  isEnabled: boolean;
  isRecording: boolean;
  isStarting: boolean;
  isTranscribing: boolean;
  toggleRecording: () => void;
}

interface DictationButtonProps {
  dictation: DictationState;
  disabled?: boolean;
}

export function DictationButton({
  dictation,
  disabled = false,
}: DictationButtonProps) {
  if (!dictation.isEnabled) return null;

  const shortcutHint = isMacPlatform() ? "⌘D" : "Ctrl+D";

  const tooltipText = dictation.isRecording
    ? "Stop recording"
    : dictation.isTranscribing
      ? "Transcribing…"
      : null;

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label={tooltipText ?? "Voice Dictation"}
          aria-pressed={dictation.isRecording}
          className={cn(
            "rounded-full",
            dictation.isRecording &&
              "bg-destructive text-destructive-foreground hover:bg-destructive/90 hover:text-destructive-foreground active:bg-destructive active:text-destructive-foreground",
            dictation.isTranscribing && "animate-pulse",
          )}
          disabled={
            dictation.isRecording ? false : disabled || dictation.isStarting
          }
          onClick={dictation.toggleRecording}
          size="icon"
          type="button"
          variant={dictation.isRecording ? "default" : "ghost"}
        >
          {dictation.isRecording ? (
            <Square className="h-3.5 w-3.5 fill-current" />
          ) : (
            <Mic />
          )}
        </Button>
      </TooltipTrigger>
      <TooltipContent>
        {tooltipText ?? (
          <span className="flex items-center gap-1.5">
            Voice Dictation
            <kbd className="rounded border border-foreground/10 px-1 py-0.5 text-2xs font-medium text-foreground/60">
              {shortcutHint}
            </kbd>
          </span>
        )}
      </TooltipContent>
    </Tooltip>
  );
}
