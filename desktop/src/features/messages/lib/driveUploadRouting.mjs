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

/**
 * Types the relay will actually reject, so they route to Drive rather than
 * failing the upload.
 *
 * **This is deliberately NOT a copy of `BLOCKED_FILE_MIME_TYPES` in
 * `crates/buzz-media/src/validation.rs`, and must not be turned into one.**
 * That list is what the relay refuses *after sniffing the bytes* with the
 * `infer` crate. Most of its entries are unreachable, because `infer` has no
 * matcher for them:
 *
 * - **SVG** sniffs as `text/xml` (or nothing) — accepted as an inert download.
 * - **JavaScript** and **XHTML** have no magic bytes — accepted.
 *   `validation.rs` says so outright for XHTML: "unreachable through sniffing".
 * - **`.msi`** sniffs as `application/x-ole-storage` — accepted.
 * - **`.apk`** sniffs as `application/zip` — accepted.
 * - **`.dmg`** has no matcher at all — accepted.
 *
 * All of those upload fine today. Routing them to Drive would take a working
 * flow and break it for anyone who has not connected a Google account, since
 * the Drive path refuses rather than falling back.
 *
 * What `infer` genuinely detects, and the relay genuinely blocks, is three
 * executable families: PE (`MZ`), ELF, and Mach-O. Only those belong here.
 *
 * `text/html` is likewise absent, and for a different reason: the relay accepts
 * it as an inert download by explicit decision. Do not add it.
 */
const RELAY_BLOCKED_MIME_TYPES = new Set([
  // PE — .exe, .dll and friends. Browsers report either of these.
  "application/x-msdownload",
  "application/vnd.microsoft.portable-executable",
  // ELF.
  "application/x-executable",
  "application/x-sharedlib",
  "application/x-elf",
  // Mach-O.
  "application/x-mach-binary",
]);

/**
 * Extensions for the same three families, since a browser `File` frequently
 * arrives with an empty or opaque MIME type.
 *
 * Cannot be exhaustive: an **extensionless** Unix or macOS binary — the common
 * case for ELF and Mach-O — is undetectable here and will take the relay path
 * and be rejected. That is the pre-existing behaviour, so nothing regresses; it
 * simply is not improved.
 */
const RELAY_BLOCKED_EXTENSIONS = new Set([
  // PE.
  "com",
  "cpl",
  "dll",
  "drv",
  "efi",
  "exe",
  "ocx",
  "scr",
  "sys",
  // ELF.
  "elf",
  "ko",
  "o",
  "out",
  "so",
  // Mach-O.
  "bundle",
  "dylib",
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
 * Whether the relay would refuse this file outright — a native executable.
 *
 * MIME first when it says something concrete, extension otherwise — the same
 * precedence `isAudioFile` and `videoMimeForFile` use. A PNG named `.exe` is a
 * PNG: the relay sniffs content, so it would accept it, and so do we.
 */
export function isRelayBlockedFile(file) {
  const type = file?.type ? String(file.type).toLowerCase() : undefined;
  if (isUsableMimeType(type)) return RELAY_BLOCKED_MIME_TYPES.has(type);
  return RELAY_BLOCKED_EXTENSIONS.has(filenameExtension(file?.name) ?? "");
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
  // Not a size judgement: these cannot go to the relay at any size.
  if (isRelayBlockedFile({ name, type })) return "drive";
  const size = Number(sizeBytes);
  if (Number.isFinite(size) && size > DRIVE_UPLOAD_THRESHOLD_BYTES) {
    return "drive";
  }
  return "relay";
}
