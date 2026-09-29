/** Stop every track on a MediaStream (best-effort cleanup after failed share). */
export function stopMediaStreamTracks(
  stream: MediaStream | null | undefined,
): void {
  if (!stream) return;
  for (const track of stream.getTracks()) {
    try {
      track.stop();
    } catch {
      /* best-effort */
    }
  }
}

/**
 * Acquire screen capture. Must be the first await in the Share click path —
 * WebKit/WKWebView requires getDisplayMedia inside a user-gesture handler.
 */
export async function acquireDisplayMedia(): Promise<MediaStream> {
  const stream = await navigator.mediaDevices.getDisplayMedia({
    video: true,
    audio: false,
  });
  if (!stream.getVideoTracks().length) {
    stopMediaStreamTracks(stream);
    throw new Error("no screen video track from getDisplayMedia");
  }
  return stream;
}
