/** Enough headroom for an eight-frame transport batch at 30 FPS. */
export const LIVE_VIDEO_TARGET_LAG_MS = 300;
/** Correct accumulated decoder/replay lag before it becomes a visible second. */
export const LIVE_VIDEO_MAX_LAG_MS = 450;
/** Only large backlogs justify interrupting native decoding with another seek. */
export const LIVE_VIDEO_SEEK_LAG_MS = 1000;

/** Keep actual presented video near the live edge, including after MSE appends. */
export function startLiveVideoPlayout(
  video: HTMLVideoElement,
  onError: (error: unknown) => void,
): () => void {
  let disposed = false;
  let frame: number | undefined;
  let started = false;
  const correct = (forceSeek = false) => {
    if (disposed || video.seeking || video.readyState < 2) return;
    try {
      const ranges = video.buffered;
      if (!ranges.length) return;
      const last = ranges.length - 1;
      const end = ranges.end(last);
      const lag = end - video.currentTime;
      if (lag > LIVE_VIDEO_MAX_LAG_MS / 1000) {
        if (!started || forceSeek || lag > LIVE_VIDEO_SEEK_LAG_MS / 1000) {
          video.playbackRate = 1;
          video.currentTime = Math.max(
            ranges.start(last),
            end - LIVE_VIDEO_TARGET_LAG_MS / 1000,
          );
        } else {
          // Frequent WKWebView seeks interrupt decoding; drain small drift smoothly.
          video.playbackRate = 1.25;
        }
      } else if (lag <= LIVE_VIDEO_TARGET_LAG_MS / 1000) {
        video.playbackRate = 1;
      }
      started = true;
    } catch (error) {
      stop();
      onError(error);
    }
  };
  const changed = () => correct();
  const visible = () => correct(true);
  const emptied = () => {
    started = false;
    video.playbackRate = 1;
  };
  const presented = () => {
    if (disposed) return;
    correct();
    if (!disposed) frame = video.requestVideoFrameCallback(presented);
  };
  const events = [
    "progress",
    "loadeddata",
    "canplay",
    "playing",
    "timeupdate",
    "seeked",
  ] as const;
  for (const event of events) video.addEventListener(event, changed);
  video.addEventListener("emptied", emptied);
  video.ownerDocument.addEventListener("visibilitychange", visible);
  if (typeof video.requestVideoFrameCallback === "function") {
    frame = video.requestVideoFrameCallback(presented);
  }
  correct();
  function stop() {
    disposed = true;
    if (frame !== undefined) video.cancelVideoFrameCallback(frame);
    for (const event of events) video.removeEventListener(event, changed);
    video.removeEventListener("emptied", emptied);
    video.ownerDocument.removeEventListener("visibilitychange", visible);
    video.playbackRate = 1;
  }
  return stop;
}
