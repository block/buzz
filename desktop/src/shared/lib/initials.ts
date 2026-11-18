import { decode } from "nostr-tools/nip19";

/**
 * Key-form labels carry no name to abbreviate: their head is the constant
 * `npub1` prefix, so name-derived initials would collapse every key-identified
 * identity onto the same leading letter. A key's distinguishing fragment is
 * its tail — the part a compact key actually shows — so derive initials from
 * there and keep key-fallback avatars visually distinct.
 *
 * Detection is deliberately narrow so authored names never lose their name
 * initials for merely resembling a key: a full label counts as a key only
 * when it decodes as a checksum-valid npub of an identity-length key, and a
 * compact label must match the exact `npub` + 4 + `…` + 4 truncation shape
 * over the bech32 alphabet (the form `truncateNpub` emits). A display name
 * that is a checksum-valid npub is indistinguishable from a real key by
 * shape alone, and taking key-tail initials there is the safe side of that
 * boundary.
 */
const BECH32_ALPHABET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const COMPACT_NPUB_KEY_LABEL = new RegExp(
  `^npub1[${BECH32_ALPHABET}]{3}…[${BECH32_ALPHABET}]{4}$`,
);
const HEX_64_REGEX = /^[0-9a-f]{64}$/;

function isFullNpubLabel(label: string): boolean {
  try {
    const decoded = decode(label);
    return decoded.type === "npub" && HEX_64_REGEX.test(decoded.data);
  } catch {
    return false;
  }
}

function keyTailInitials(name: string): string | null {
  const trimmed = name.trim();
  if (!COMPACT_NPUB_KEY_LABEL.test(trimmed) && !isFullNpubLabel(trimmed)) {
    return null;
  }
  return trimmed.slice(-4, -2).toUpperCase();
}

/** Derive up to two uppercase initials from a display name. */
export function getInitials(name: string): string {
  const keyInitials = keyTailInitials(name);
  if (keyInitials !== null) {
    return keyInitials;
  }
  return name
    .replace(/[^\p{L}\p{N}\s]/gu, " ")
    .trim()
    .split(/\s+/)
    .map((part) => part[0] ?? "")
    .join("")
    .slice(0, 2)
    .toUpperCase();
}
