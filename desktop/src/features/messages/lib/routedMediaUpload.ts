import { uploadMediaFile as uploadToRelay } from "@/shared/api/tauriMedia";
import {
  getGoogleDriveStatus,
  uploadFileToDrive,
} from "@/shared/api/tauriDrive";

import { uploadRouteFor } from "./driveUploadRouting.mjs";
import type { ImetaMedia } from "./imetaMediaMarkdown";
import { isVideoFile } from "./videoFileType";

/**
 * Drop-in replacement for `uploadMediaFile` that sends large files, video and
 * audio to the sender's Google Drive instead of the relay.
 *
 * Deliberately kept signature-compatible so `useMediaUpload` and
 * `backgroundMediaUploadStore` swap one import and keep every other behaviour
 * they already have — cancellation, progress, slot ordering, the deferred
 * video queue. The routing rule itself lives in `driveUploadRouting.mjs`, pure
 * and tested; this module is only the plumbing.
 *
 * ## Why a Drive upload still returns a `BlobDescriptor`
 *
 * It has to, to travel through the composer's attachment state unchanged. But
 * it is marked `external`, which `imetaMediaMarkdown.ts` reads to do two
 * things: emit no `imeta` tag for it, and always render it as a plain
 * `[filename](url)` link rather than an inline player.
 *
 * The result is that a Drive upload posts exactly what a person pasting a
 * labelled link would post — no fabricated `imeta` claiming a sha256 and a
 * relay blob that do not exist, and no `<video>` element pointed at a Drive
 * viewer page that will never play. The Files tab then picks it up as a link
 * entry named after the file, because `channelLinkEntries.mjs` reads the
 * markdown label. That is the whole reason no Drive API call is needed to name
 * a file Buzz itself uploaded.
 */
/**
 * Whether Drive routing is live.
 *
 * Off under e2e unless a spec asks for it, mirroring how
 * `deferredComposerUploads` is gated in `useMediaUpload`. The e2e suite has no
 * Google account, and several specs deliberately upload a 16 MB file or a video
 * to assert the *relay* path — routing those to Drive would fail them for a
 * reason that has nothing to do with what they test. The Drive path is covered
 * by unit tests over `driveUploadRouting.mjs` and, ultimately, by using it.
 */
function driveRoutingEnabled(): boolean {
  const e2e = (
    window as Window & {
      __BUZZ_E2E__?: { mock?: { driveUploads?: boolean } };
    }
  ).__BUZZ_E2E__;
  return !e2e || e2e.mock?.driveUploads === true;
}

export async function uploadMediaFile(
  file: File,
  progressId?: string,
  signal?: AbortSignal,
  onDispatch?: () => void,
): Promise<ImetaMedia> {
  if (!driveRoutingEnabled()) {
    return uploadToRelay(file, progressId, signal, onDispatch);
  }

  const route = uploadRouteFor({
    isVideo: isVideoFile(file),
    name: file.name,
    sizeBytes: file.size,
    type: file.type,
  });
  if (route === "relay") {
    return uploadToRelay(file, progressId, signal, onDispatch);
  }

  // Refuse rather than silently falling back to the relay. Falling back would
  // reintroduce exactly the failure this routes around, and for video it would
  // fail anyway — the transcode needs ffmpeg on this machine.
  if (!(await getGoogleDriveStatus())) {
    throw new Error(
      "Video, audio, programs, and files over 5 MB are shared through your Google Drive. Connect your Google account under Settings → Voice to send this.",
    );
  }

  if (signal?.aborted) throw new Error("upload cancelled");
  onDispatch?.();
  const uploaded = await uploadFileToDrive(file, progressId);
  if (signal?.aborted) throw new Error("upload cancelled");

  return {
    external: true,
    filename: uploaded.name,
    sha256: "",
    size: file.size,
    // Never a `video/*` or `image/*` type, so nothing downstream tries to
    // render a Drive viewer page inline. The real type is not lost — Drive
    // holds it, and the filename still carries the extension.
    type: "application/octet-stream",
    uploaded: 0,
    url: uploaded.webViewLink,
  };
}
