/**
 * Interim caption-language convention: translator bots (e.g. `caption_forward.py`)
 * prefix translated captions with a `[XX]` banner (`[ES]`, `[ZH]`, ...) since
 * kind:9 has no structured language tag yet. This is a stand-in for the
 * proposed `l`-tag; swap `parseCaptionLanguageBanner` for tag parsing once
 * that lands.
 */
export function parseCaptionLanguageBanner(content: string): string | null {
  const match = /^\[([A-Za-z]{2,3})\]/.exec(content.trim());
  return match ? match[1].toLowerCase() : null;
}

/**
 * Whether a kind:9 message should be spoken aloud for a listener with the
 * given caption preferences. Messages without a recognized language banner
 * (ordinary agent replies, not translated captions) are always eligible —
 * only banner-tagged captions in another language are held back. Captions
 * always render as text regardless of this result.
 */
export function shouldSpeakCaption(
  content: string,
  speakCaptions: boolean,
  captionLanguage: string,
): boolean {
  if (!speakCaptions) return false;
  const bannerLanguage = parseCaptionLanguageBanner(content);
  if (bannerLanguage === null) return true;
  return bannerLanguage === captionLanguage.toLowerCase();
}
