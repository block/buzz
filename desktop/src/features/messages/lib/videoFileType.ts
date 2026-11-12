/**
 * Video detection for composer attachments, kept DOM-free so the branch logic
 * is unit-testable without a webview.
 *
 * The deferred-upload split routes videos to the queued/background path and
 * everything else to an immediate foreground upload, so this predicate decides
 * which path a file takes. A MIME-only check is not enough: file-picker,
 * drag/drop, and clipboard `File` objects can arrive with an empty or generic
 * `application/octet-stream` type for perfectly valid videos (no OS MIME
 * database entry, network shares, some Linux desktops), and those videos would
 * otherwise upload in the foreground and block Send until the transcode
 * finishes.
 *
 * A concrete MIME type stays authoritative — `image/gif` named `clip.mp4` is
 * an image. The filename extension is consulted only when the MIME type tells
 * us nothing.
 */

/** Extension → MIME, used only when a file carries no usable MIME type. */
const VIDEO_MIME_BY_EXTENSION = new Map([
  ["avi", "video/x-msvideo"],
  ["m4v", "video/mp4"],
  ["mkv", "video/x-matroska"],
  ["mov", "video/quicktime"],
  ["mp4", "video/mp4"],
  ["webm", "video/webm"],
]);

/** MIME types that carry no format information — treat as "unknown". */
const OPAQUE_MIME_TYPES = new Set([
  "application/octet-stream",
  "binary/octet-stream",
]);

function isUsableMimeType(type: string | undefined): boolean {
  if (!type) return false;
  return !OPAQUE_MIME_TYPES.has(type.toLowerCase());
}

/** The lowercased extension of a filename, or undefined when it has none. */
function filenameExtension(filename: string | undefined): string | undefined {
  if (!filename) return undefined;
  const lastDot = filename.lastIndexOf(".");
  if (lastDot <= 0 || lastDot === filename.length - 1) return undefined;
  return filename.slice(lastDot + 1).toLowerCase();
}

type VideoFileCandidate = {
  name?: string;
  type?: string;
};

/**
 * The video MIME type to use for `file`, or undefined when it is not a video.
 *
 * Returns the file's own MIME type when it is a usable `video/*` type, and
 * otherwise falls back to an extension-derived type for files whose MIME is
 * missing or opaque.
 */
export function videoMimeForFile(file: VideoFileCandidate): string | undefined {
  const type = file.type?.toLowerCase();
  if (isUsableMimeType(type)) {
    return type?.startsWith("video/") ? type : undefined;
  }
  return VIDEO_MIME_BY_EXTENSION.get(filenameExtension(file.name) ?? "");
}

/** Whether `file` should be treated as a video by the composer. */
export function isVideoFile(file: VideoFileCandidate): boolean {
  return videoMimeForFile(file) !== undefined;
}
