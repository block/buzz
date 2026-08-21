/** Files larger than this go to Drive rather than the relay. 5 MB. */
export const DRIVE_UPLOAD_THRESHOLD_BYTES: number;

/** Enough of a `File` to decide where it should go. */
export type UploadRouteCandidate = {
  name?: string;
  type?: string;
  sizeBytes?: number;
  /**
   * From `videoFileType.ts`. Passed in rather than derived here so there is
   * only ever one answer to "is this a video" in the codebase.
   */
  isVideo?: boolean;
};

/**
 * Whether a file should be treated as audio.
 *
 * A concrete MIME type is authoritative; the filename extension is consulted
 * only when the MIME type is missing or opaque.
 */
export function isAudioFile(file: { name?: string; type?: string }): boolean;

/** Where one attachment goes. */
export function uploadRouteFor(
  candidate: UploadRouteCandidate,
): "drive" | "relay";
