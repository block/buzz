export const WAVE_MESSAGE_MARKER = "<!-- buzz:wave:v1 -->";

export type WaveMessageContent = {
  fallbackText: string;
};

export function buildWaveMessageContent(senderName: string): string {
  const trimmedName = senderName.trim() || "Someone";
  return `${WAVE_MESSAGE_MARKER}\n${trimmedName} waved at you.`;
}

export function parseWaveMessageContent(
  content: string,
): WaveMessageContent | null {
  const trimmedContent = content.trimStart();

  if (!trimmedContent.startsWith(WAVE_MESSAGE_MARKER)) {
    return null;
  }

  const fallbackText = trimmedContent.slice(WAVE_MESSAGE_MARKER.length).trim();

  return {
    fallbackText: fallbackText || "Someone waved at you.",
  };
}
