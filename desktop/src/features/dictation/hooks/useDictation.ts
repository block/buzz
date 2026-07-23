import type * as React from "react";
import { useCallback, useMemo, useRef } from "react";
import {
  DEFAULT_AUTO_SUBMIT_PHRASE,
  getAutoSubmitMatch,
  parseAutoSubmitPhrases,
  replaceTrailingTranscribedText,
} from "../lib/voiceInput";
import { useLocalDictation } from "./useLocalDictation";

interface UseDictationOptions {
  /** Disable native availability checks and recording entry points. */
  disabled?: boolean;
  /** Returns the current composer text (must be fresh — synced from editor). */
  getText: () => string;
  /** Set composer text */
  setText: (value: string) => void;
  /** Send the message */
  onSend: (text: string) => void;
  /** Ref that is `true` when sending is blocked (uploading, preparing mention, etc.) */
  isSendBlockedRef?: React.MutableRefObject<boolean>;
}

export function useDictation({
  disabled = false,
  getText,
  setText,
  onSend,
  isSendBlockedRef,
}: UseDictationOptions) {
  const autoSubmitPhrases = useMemo(
    () => parseAutoSubmitPhrases(DEFAULT_AUTO_SUBMIT_PHRASE),
    [],
  );
  const stopRecordingRef = useRef<() => void>(() => {});
  const lastTranscriptRef = useRef("");

  const handleTranscript = useCallback(
    (transcript: string) => {
      const previous = lastTranscriptRef.current;
      const latest = getText();
      const merged = replaceTrailingTranscribedText(
        latest,
        previous,
        transcript,
      );
      const match = getAutoSubmitMatch(transcript, autoSubmitPhrases);

      if (!match) {
        setText(merged);
        // Reset to empty — each streaming partial is an independent segment
        // (the native engine flushes and clears its buffer). The next transcript
        // should be appended, not replace this one.
        lastTranscriptRef.current = "";
        return;
      }

      const textWithoutPhrase = replaceTrailingTranscribedText(
        latest,
        previous,
        match.textWithoutPhrase,
      );
      if (!textWithoutPhrase.trim()) return;

      stopRecordingRef.current();

      if (isSendBlockedRef?.current) {
        setText(textWithoutPhrase);
        return;
      }

      setText(textWithoutPhrase.trim());
      onSend(textWithoutPhrase.trim());
      lastTranscriptRef.current = "";
    },
    [autoSubmitPhrases, getText, onSend, isSendBlockedRef, setText],
  );

  const dictation = useLocalDictation({
    disabled,
    onRecordingStart: () => {
      lastTranscriptRef.current = "";
    },
    onTranscriptText: handleTranscript,
  });

  stopRecordingRef.current = dictation.stopRecording;

  return dictation;
}
