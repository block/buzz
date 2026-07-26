/**
 * Stable per-author accent colour for transcript user messages.
 *
 * A shared harness session can have several people prompting the same agent, so
 * "who said this" has to be readable while scrolling past. Colour is derived
 * from the author's pubkey rather than assigned by join order, so the same
 * person is the same colour in every client and across restarts — no shared
 * state to keep in sync.
 *
 * Saturation and lightness are fixed and mid-range so the hue reads on both the
 * light and dark themes; only the hue varies.
 */

const HUE_STEPS = 360;

/** Deterministic 0–359 hue from a pubkey (FNV-1a, so no crypto dependency). */
export function authorHue(pubkey: string | null | undefined): number {
  if (!pubkey) {
    return 0;
  }
  let hash = 0x811c9dc5;
  for (let index = 0; index < pubkey.length; index += 1) {
    hash ^= pubkey.charCodeAt(index);
    // FNV prime, kept in 32-bit range via Math.imul.
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash % HUE_STEPS;
}

export type AuthorAccent = {
  /** Left rule and author-name colour. */
  border: string;
  /** Faint fill so the message block is distinguishable at a glance. */
  background: string;
};

export function authorAccent(pubkey: string | null | undefined): AuthorAccent {
  const hue = authorHue(pubkey);
  return {
    border: `hsl(${hue} 70% 55%)`,
    background: `hsl(${hue} 70% 55% / 0.10)`,
  };
}
