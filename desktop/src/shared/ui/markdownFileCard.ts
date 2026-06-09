import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

/** Minimal shape of an imeta entry as consumed by the markdown renderer. */
export type FileCardImetaEntry = {
  m?: string;
  size?: number;
  filename?: string;
};

export type ResolvedFileCard = {
  href: string;
  filename: string;
  size?: number;
};

/**
 * Decide whether a markdown link should render as a generic-file download
 * card. A link qualifies when its href matches an imeta entry whose MIME is
 * neither image nor video (media goes through the `img` renderer instead).
 *
 * Pure — extracted from `markdown.tsx` so the FileCard decision (the riskiest
 * part of the generic-file rendering path) is unit-testable without mounting
 * React. Returns the resolved card props, or `null` to fall through to normal
 * link handling.
 */
export function resolveFileCard(
  entry: FileCardImetaEntry | undefined,
  href: string | undefined,
  childText: string,
): ResolvedFileCard | null {
  if (
    !href ||
    !entry?.m ||
    entry.m.startsWith("image/") ||
    entry.m.startsWith("video/")
  ) {
    return null;
  }
  const filename =
    entry.filename || childText.trim() || href.split("/").pop() || "file";
  return { href: rewriteRelayUrl(href), filename, size: entry.size };
}
