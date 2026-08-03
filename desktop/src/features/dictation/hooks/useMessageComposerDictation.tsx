import type * as React from "react";
import { useCallback, useEffect, useRef } from "react";
import { useFeatureEnabled } from "@/shared/features";
import {
  getDictationSendDecision,
  shouldAutoSubmitDictation,
} from "../lib/voiceInput";
import { DictationButton } from "../ui/DictationButton";
import { useComposerDictation } from "./useComposerDictation";

interface UseMessageComposerDictationOptions {
  syncContentRef: React.MutableRefObject<() => string>;
  disabled: boolean;
  disabledRef: React.MutableRefObject<boolean>;
  isSendingRef: React.MutableRefObject<boolean>;
  isUploadingRef: React.MutableRefObject<boolean>;
  setComposerContent: (text: string) => void;
  setEditorContent: (text: string) => void;
  draftKey: string | null;
  composerRef: React.RefObject<HTMLElement | null>;
  submitMessageRef: React.MutableRefObject<() => void>;
}

export function useMessageComposerDictation({
  disabled,
  draftKey,
  setEditorContent,
  submitMessageRef,
  ...options
}: UseMessageComposerDictationOptions) {
  const enabled = useFeatureEnabled("voiceDictation");
  const setEditorContentRef = useRef(setEditorContent);
  setEditorContentRef.current = setEditorContent;
  const dictation = useComposerDictation({
    ...options,
    disabled,
    draftKey,
    enabled,
    setEditorContentRef,
  });
  const { isRecording, isStarting, isTranscribing, stopRecording } = dictation;
  const sendAfterTranscriptionRef = useRef(false);

  // biome-ignore lint/correctness/useExhaustiveDependencies: composer scope and availability are the reset triggers for a queued send
  useEffect(() => {
    sendAfterTranscriptionRef.current = false;
  }, [disabled, draftKey]);

  useEffect(() => {
    if (
      !shouldAutoSubmitDictation({
        requested: sendAfterTranscriptionRef.current,
        isRecording,
        isStarting,
        isTranscribing,
      })
    ) {
      return;
    }
    sendAfterTranscriptionRef.current = false;
    submitMessageRef.current();
  }, [isRecording, isStarting, isTranscribing, submitMessageRef]);

  const prepareToSubmit = useCallback(() => {
    const decision = getDictationSendDecision({
      isRecording,
      isStarting,
      isTranscribing,
    });
    if (decision !== "send") {
      sendAfterTranscriptionRef.current = true;
    }
    if (decision === "stop-recording") {
      stopRecording();
    }
    return decision === "send";
  }, [isRecording, isStarting, isTranscribing, stopRecording]);

  return {
    ...dictation,
    canStopFromSend: isRecording || isStarting,
    prepareToSubmit,
  };
}

export function MessageComposerDictationAction({
  children,
  dictation,
  disabled,
}: {
  children?: React.ReactNode;
  dictation: ReturnType<typeof useMessageComposerDictation>;
  disabled: boolean;
}) {
  return (
    <>
      <DictationButton dictation={dictation} disabled={disabled} />
      {children}
    </>
  );
}
