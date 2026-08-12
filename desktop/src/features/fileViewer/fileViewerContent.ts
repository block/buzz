/**
 * Byte-level decision for the file viewer: is a fetched attachment previewable
 * text, and if so what is its text?
 *
 * Pure and separate from the fetching hook so the checks that keep untrusted
 * bytes out of the renderer are unit-testable.
 */

/**
 * Largest preview we decode. Well under the 50 MiB IPC transfer cap: a
 * multi-megabyte string through react-markdown or Shiki would freeze the UI.
 */
export const MAX_FILE_PREVIEW_BYTES = 2 * 1024 * 1024;

/** Bytes sniffed for a NUL before deciding a file is binary. */
const BINARY_SNIFF_BYTES = 8192;

export type FileViewerContent =
  | { status: "loading" }
  | { status: "error"; message: string }
  | { status: "too-large" }
  | { status: "binary" }
  | { status: "text"; text: string };

export type DecodedFileViewerContent = Extract<
  FileViewerContent,
  { status: "too-large" | "binary" | "text" }
>;

/**
 * Classify fetched attachment bytes.
 *
 * A NUL byte in the head means binary — the same sniff git uses. The imeta
 * MIME is sender-controlled, so the decision comes from the bytes.
 */
export function decodeFileViewerContent(
  bytes: Uint8Array,
): DecodedFileViewerContent {
  if (bytes.byteLength > MAX_FILE_PREVIEW_BYTES) {
    return { status: "too-large" };
  }
  if (bytes.subarray(0, BINARY_SNIFF_BYTES).includes(0)) {
    return { status: "binary" };
  }
  return { status: "text", text: new TextDecoder("utf-8").decode(bytes) };
}
