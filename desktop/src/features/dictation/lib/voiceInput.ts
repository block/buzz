export function appendTranscribedText(
  baseText: string,
  fragment: string,
): string {
  const normalizedFragment = fragment.replace(/\s+/g, " ").trim();
  if (!normalizedFragment) return baseText;
  if (!baseText.trim()) return normalizedFragment;
  if (/[\s([{/-]$/.test(baseText) || /^[,.;!?)]/.test(normalizedFragment)) {
    return `${baseText}${normalizedFragment}`;
  }
  return `${baseText} ${normalizedFragment}`;
}

export type DictationSendDecision = "send" | "stop-recording" | "wait";

export function getDictationSendDecision({
  isRecording,
  isStarting,
  isTranscribing,
}: {
  isRecording: boolean;
  isStarting: boolean;
  isTranscribing: boolean;
}): DictationSendDecision {
  if (isRecording || isStarting) return "stop-recording";
  if (isTranscribing) return "wait";
  return "send";
}

export function shouldAutoSubmitDictation({
  requested,
  isRecording,
  isStarting,
  isTranscribing,
}: {
  requested: boolean;
  isRecording: boolean;
  isStarting: boolean;
  isTranscribing: boolean;
}): boolean {
  return requested && !isRecording && !isStarting && !isTranscribing;
}
