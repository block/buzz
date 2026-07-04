import { Mic } from "lucide-react";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { cn } from "@/shared/lib/cn";

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

  const tooltipText = dictation.isRecording
    ? "Stop recording"
    : dictation.isTranscribing
      ? "Transcribing…"
      : "Dictate message";

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label={tooltipText}
          aria-pressed={dictation.isRecording}
          className={cn(
            "rounded-full",
            dictation.isRecording &&
              "bg-destructive text-destructive-foreground hover:bg-destructive/90 hover:text-destructive-foreground active:bg-destructive active:text-destructive-foreground",
            dictation.isTranscribing && "animate-pulse",
          )}
          disabled={disabled || dictation.isStarting}
          onClick={dictation.toggleRecording}
          size="icon"
          type="button"
          variant={dictation.isRecording ? "default" : "ghost"}
        >
          <Mic />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{tooltipText}</TooltipContent>
    </Tooltip>
  );
}
