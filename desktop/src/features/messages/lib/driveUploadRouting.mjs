/**
 * Where a composer attachment goes: the relay, or the sender's Google Drive.
 *
 * Two reasons a file takes the Drive path, and they are different reasons:
 *
 * - **Video and audio, always.** Not a size judgement. The relay path
 *   transcodes video locally through ffmpeg, which is not preinstalled on
 *   macOS, and then lands on a hosted relay whose caps this fork does not
 *   control. A three-second voice note takes this path for the same reason a
 *   two-hour recording does — the path itself is the problem.
 * - **Anything over the size threshold.** A blunt rule about not pushing large
 *   files through someone else's storage budget.
 *
 * The threshold is a constant, not a setting. One place to change it, no
 * settings surface to build, and it can be tuned once there is real usage to
 * tune against. It is deliberately equal to the point where Drive's simple
 * upload endpoint stops working, which is why the Rust side uses the resumable
 * endpoint for everything on this path.
 *
 * Video detection is NOT reimplemented here — the caller passes `isVideo` from
 * `videoFileType.ts`, which already owns that decision and is used by the
 * deferred-upload split. Two answers to "is this a video" would eventually
 * disagree.
 */

/** 5 MB. Files larger than this go to Drive. */
export const DRIVE_UPLOAD_THRESHOLD_BYTES = 5 * 1024 * 1024;

/** Extension → MIME for audio, consulted only when the MIME says nothing. */
const AUDIO_MIME_BY_EXTENSION = new Map([
  ["aac", "audio/aac"],
  ["flac", "audio/flac"],
  ["m4a", "audio/mp4"],
  ["mp3", "audio/mpeg"],
  ["oga", "audio/ogg"],
  ["ogg", "audio/ogg"],
  ["opus", "audio/opus"],
  ["wav", "audio/wav"],
  ["wma", "audio/x-ms-wma"],
]);

/** MIME types that carry no format information — treat as "unknown". */
const OPAQUE_MIME_TYPES = new Set([
  "application/octet-stream",
  "binary/octet-stream",
]);

function isUsableMimeType(type) {
  if (!type) return false;
  return !OPAQUE_MIME_TYPES.has(String(type).toLowerCase());
}

/** The lowercased extension of a filename, or undefined when it has none. */
function filenameExtension(filename) {
  if (!filename) return undefined;
  const name = String(filename);
  const lastDot = name.lastIndexOf(".");
  if (lastDot <= 0 || lastDot === name.length - 1) return undefined;
  return name.slice(lastDot + 1).toLowerCase();
}

/**
 * Whether a file should be treated as audio.
 *
 * Mirrors `videoMimeForFile`'s precedence deliberately: a concrete MIME type
 * is authoritative, and the extension is consulted only when the MIME type is
 * missing or opaque. File-picker, drag-drop and clipboard `File` objects
 * routinely arrive with an empty or `application/octet-stream` type for
 * perfectly valid media.
 */
export function isAudioFile(file) {
  const type = file?.type ? String(file.type).toLowerCase() : undefined;
  if (isUsableMimeType(type)) return Boolean(type?.startsWith("audio/"));
  return AUDIO_MIME_BY_EXTENSION.has(filenameExtension(file?.name) ?? "");
}

/**
 * `"drive"` or `"relay"` for one attachment.
 *
 * `isVideo` comes from `videoFileType.ts`; pass it rather than letting this
 * module guess.
 */
export function uploadRouteFor({ name, type, sizeBytes, isVideo = false }) {
  if (isVideo) return "drive";
  if (isAudioFile({ name, type })) return "drive";
  const size = Number(sizeBytes);
  if (Number.isFinite(size) && size > DRIVE_UPLOAD_THRESHOLD_BYTES) {
    return "drive";
  }
  return "relay";
}
