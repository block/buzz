/**
 * Pure helpers for the inline video right-click menu, kept out of the
 * component so the branch logic is unit-testable without a DOM.
 */

/** Fallback save-dialog name when the imeta `filename` field is absent. */
export const DEFAULT_VIDEO_FILENAME = "video.mp4";

/**
 * The suggested filename passed to the native `download_file` command.
 *
 * The relay only stores a content hash in the URL path, so the imeta
 * `filename` is the only human-readable name we have. When it is missing we
 * fall back to a generic `.mp4` name (the backend re-derives the real
 * extension defensively from this value).
 */
export function resolveVideoDownloadFilename(filename?: string): string {
  const trimmed = filename?.trim();
  return trimmed ? trimmed : DEFAULT_VIDEO_FILENAME;
}
