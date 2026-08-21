import { invoke as invokeTauriRaw } from "@tauri-apps/api/core";
import { invokeTauri } from "./tauri";

/** What Drive gave back for an uploaded file. */
export type DriveUpload = {
  fileId: string;
  /** The name Drive stored, used as the message's link label. */
  name: string;
  /** A Drive viewer page, not a direct byte URL. */
  webViewLink: string;
};

/**
 * Base64url, matching `decode_header` in `google_meet/drive.rs`.
 *
 * IPC headers are ASCII-only, and filenames are not — `quarterly review 🎬.mp4`
 * has to survive the trip intact, since it becomes the link label people read
 * in the Files tab.
 */
function encodeRawIpcHeader(value: string): string {
  const bytes = new TextEncoder().encode(value);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return window
    .btoa(binary)
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replace(/=+$/, "");
}

/**
 * True when a Google account is connected **and** granted the Drive scope.
 *
 * Distinct from the Meet connection status: anyone who connected before Drive
 * shipped is connected for Meet and not for Drive, and needs to reconnect.
 */
export async function getGoogleDriveStatus(): Promise<boolean> {
  return invokeTauri<boolean>("get_google_drive_status", {});
}

/**
 * Upload a file to the sender's own Drive and return a shareable link.
 *
 * Bytes cross the IPC boundary raw rather than as JSON, the same transport
 * `uploadMediaFile` uses — a 200 MB video expanded into a JSON array would be
 * roughly four times the size and would have to be parsed as one string.
 */
export async function uploadFileToDrive(
  file: File,
  progressId?: string,
): Promise<DriveUpload> {
  const headers: Record<string, string> = {
    "x-buzz-filename": encodeRawIpcHeader(file.name),
    "x-buzz-mime": encodeRawIpcHeader(file.type || "application/octet-stream"),
  };
  if (progressId) {
    headers["x-buzz-progress-id"] = encodeRawIpcHeader(progressId);
  }

  const bytes = new Uint8Array(await file.arrayBuffer());
  return invokeTauriRaw<DriveUpload>("upload_drive_bytes_raw", bytes, {
    headers,
  });
}
