export function resolveActivityChannelId(
  slides: string[],
  selectedChannelId: string | null,
  preferredChannelId: string | null,
): string | null {
  if (selectedChannelId && slides.includes(selectedChannelId)) {
    return selectedChannelId;
  }
  if (preferredChannelId && slides.includes(preferredChannelId)) {
    return preferredChannelId;
  }
  return slides[0] ?? null;
}
