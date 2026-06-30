export const WAVE_MESSAGE_MARKER = "<!-- buzz:wave:v1 -->";
const WAVE_TARGET_PREFIX = "<!-- buzz:wave-target:";
const WAVE_TARGET_SUFFIX = "-->";
const WAVE_TARGET_AGENT_LINE = "<!-- buzz:wave-target-agent:1 -->";

export type WaveMessageContent = {
  fallbackText: string;
  targetPubkey?: string;
  targetIsAgent?: boolean;
};

export function buildWaveMessageContent(
  senderName: string,
  targetPubkey?: string,
  options?: { targetIsAgent?: boolean },
): string {
  const trimmedName = senderName.trim() || "Someone";
  const targetLine = targetPubkey
    ? `\n${WAVE_TARGET_PREFIX}${targetPubkey.toLowerCase()}${WAVE_TARGET_SUFFIX}`
    : "";
  const targetAgentLine = options?.targetIsAgent
    ? `\n${WAVE_TARGET_AGENT_LINE}`
    : "";
  return `${WAVE_MESSAGE_MARKER}${targetLine}${targetAgentLine}\n${trimmedName} waved at you.`;
}

export function parseWaveMessageContent(
  content: string,
): WaveMessageContent | null {
  const trimmedContent = content.trimStart();

  if (!trimmedContent.startsWith(WAVE_MESSAGE_MARKER)) {
    return null;
  }

  let body = trimmedContent.slice(WAVE_MESSAGE_MARKER.length).trim();
  let targetPubkey: string | undefined;
  let targetIsAgent = false;
  if (body.startsWith(WAVE_TARGET_PREFIX)) {
    const suffixIndex = body.indexOf(WAVE_TARGET_SUFFIX);
    if (suffixIndex >= 0) {
      targetPubkey = body
        .slice(WAVE_TARGET_PREFIX.length, suffixIndex)
        .trim()
        .toLowerCase();
      body = body.slice(suffixIndex + WAVE_TARGET_SUFFIX.length).trim();
    }
  }
  if (body.startsWith(WAVE_TARGET_AGENT_LINE)) {
    targetIsAgent = true;
    body = body.slice(WAVE_TARGET_AGENT_LINE.length).trim();
  }

  return {
    fallbackText: body || "Someone waved at you.",
    targetPubkey,
    targetIsAgent,
  };
}

export function collectWaveTargetPubkeys(
  events: Array<{ content?: string }>,
): string[] {
  const pubkeys = new Set<string>();

  for (const event of events) {
    const waveMessage = parseWaveMessageContent(event.content ?? "");
    if (waveMessage?.targetPubkey) {
      pubkeys.add(waveMessage.targetPubkey);
    }
  }

  return [...pubkeys];
}
