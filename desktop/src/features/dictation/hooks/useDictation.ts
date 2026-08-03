import { useCallback } from "react";
import { appendTranscribedText } from "../lib/voiceInput";
import { useLocalDictation } from "./useLocalDictation";

interface UseDictationOptions {
  /** Disable native availability checks and recording entry points. */
  disabled?: boolean;
  /** Returns the current composer text (must be fresh — synced from editor). */
  getText: () => string;
  /** Set composer text */
  setText: (value: string) => void;
}

export function useDictation({
  disabled = false,
  getText,
  setText,
}: UseDictationOptions) {
  const handleTranscript = useCallback(
    (transcript: string) => {
      setText(appendTranscribedText(getText(), transcript));
    },
    [getText, setText],
  );

  const dictation = useLocalDictation({
    disabled,
    onTranscriptText: handleTranscript,
  });

  return dictation;
}
