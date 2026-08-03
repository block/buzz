function normalizeVoiceText(value: string): string {
  return value
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim()
    .replace(/\s+/g, " ");
}

export function shouldForwardVoiceTurn(
  transcript: string,
  agentName: string,
): boolean {
  const normalizedAgent = normalizeVoiceText(agentName);
  if (!normalizedAgent) return false;
  if (normalizedAgent === "orchestrator") return true;

  const normalizedTranscript = normalizeVoiceText(transcript);
  if (!normalizedTranscript) return false;

  const agent = escapeRegExp(normalizedAgent);
  const conciseQualifier =
    "(?:briefly|quickly|in (?:a|one) sentence|in (?:a few|one) words?)";
  const directAddress = new RegExp(
    `^(?:(?:hey|okay|ok|please) )?${agent}(?:$| (?:(?:please )?(?:ask|tell|have|let|bring|invite|remove|start|stop|mute|unmute|join|leave|implement|review|check|explain|answer|help|handle|take|look)|${conciseQualifier}|(?:what|how|why|when|where|can|could|would|should|will|do))(?: |$))`,
  );
  const directRequest = new RegExp(
    `^(?:please )?(?:ask|tell|have|let|bring|invite|remove|start|stop|mute|unmute) ${agent}(?: |$)`,
  );
  const directQuestion = new RegExp(
    `^(?:(?:what|how|why|when|where) (?:does|would|should|can|could|is)|(?:can|could|would|should|will|is)) ${agent}(?: |$)`,
  );
  const delegatedRequest = new RegExp(
    `^(?:can|could|would|will) (?:you |we )?(?:ask|tell|have|let|bring|invite|remove|start|stop|mute|unmute) ${agent}(?: |$)`,
  );

  return (
    directAddress.test(normalizedTranscript) ||
    directRequest.test(normalizedTranscript) ||
    directQuestion.test(normalizedTranscript) ||
    delegatedRequest.test(normalizedTranscript)
  );
}

export function resolveVoiceTurnRecipient<
  T extends {
    agentName: string;
    threadId: string;
  },
>(transcript: string, targets: readonly T[]): T | null {
  const specialist = targets.find(
    (target) =>
      normalizeVoiceText(target.agentName) !== "orchestrator" &&
      shouldForwardVoiceTurn(transcript, target.agentName),
  );
  if (specialist) return specialist;
  return (
    targets.find(
      (target) => normalizeVoiceText(target.agentName) === "orchestrator",
    ) ??
    targets[0] ??
    null
  );
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
