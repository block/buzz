import { HuddleMediaClock } from "./huddleMediaClock";

/** Origin-local camera selection; device identifiers are never published. */
export type CameraDevice = { deviceId: string; label: string };
/** Maximum camera capture width before native encoding. */
export const CAMERA_WIDTH = 1280;
/** Maximum camera capture height before native encoding. */
export const CAMERA_HEIGHT = 720;
/** Target cadence; transport batches contain multiple video frames. */
export const CAMERA_FPS = 30;

/** Detection never opens a camera or requests microphone access. */
export async function listHuddleCameras(
  devices: Pick<MediaDevices, "enumerateDevices">,
): Promise<CameraDevice[]> {
  const found = await devices.enumerateDevices();
  return found
    .filter((device) => device.kind === "videoinput")
    .slice(0, 16)
    .map((device, index) => ({
      deviceId: device.deviceId,
      label: device.label || `Camera ${index + 1}`,
    }));
}

function constraintFailure(error: unknown) {
  const failure = error as { name?: string; message?: string } | null;
  return (
    failure?.name === "OverconstrainedError" ||
    failure?.name === "ConstraintNotSatisfiedError" ||
    failure?.message === "Invalid constraint"
  );
}

/** Opt-in video only; a minimal retry never substitutes a selected camera. */
export async function acquireHuddleCamera(
  devices: Pick<MediaDevices, "getUserMedia" | "getSupportedConstraints">,
  deviceId: string,
  isCurrent: () => boolean,
): Promise<MediaStream> {
  const supported = devices.getSupportedConstraints();
  const video: MediaTrackConstraints = {};
  if (supported.width) video.width = { ideal: CAMERA_WIDTH };
  if (supported.height) video.height = { ideal: CAMERA_HEIGHT };
  if (supported.frameRate)
    video.frameRate = { ideal: CAMERA_FPS, max: CAMERA_FPS };
  if (deviceId) video.deviceId = { exact: deviceId };
  if (!isCurrent()) throw new Error("Camera request was cancelled");
  let stream: MediaStream;
  try {
    stream = await devices.getUserMedia({ audio: false, video });
  } catch (error) {
    if (!isCurrent()) throw new Error("Camera request was cancelled");
    if (!constraintFailure(error)) throw error;
    stream = await devices.getUserMedia({
      audio: false,
      video: deviceId ? { deviceId: { exact: deviceId } } : true,
    });
  }
  if (!isCurrent() || stream.getVideoTracks().length === 0) {
    stream.getTracks().forEach((track) => {
      track.stop();
    });
    throw new Error(
      isCurrent()
        ? "Camera returned no video track"
        : "Camera request was cancelled",
    );
  }
  return stream;
}

/** One raw frame in flight; camera frames are encoded by the native H.264 engine. */
export async function startCameraFramePump({
  stream,
  isCurrent,
  submit,
  fail,
}: {
  stream: MediaStream;
  isCurrent: () => boolean;
  submit: (data: Uint8Array, width: number, height: number) => Promise<unknown>;
  fail: (error: unknown) => void;
}): Promise<() => void> {
  const video = document.createElement("video");
  video.muted = true;
  video.playsInline = true;
  video.srcObject = stream;
  const canvas = document.createElement("canvas");
  const availableContext = canvas.getContext("2d", {
    willReadFrequently: true,
  });
  if (!availableContext) {
    video.srcObject = null;
    throw new Error("Camera pixel capture is unavailable");
  }
  const context: CanvasRenderingContext2D = availableContext;
  let disposed = false;
  let clock: HuddleMediaClock | undefined;
  const tracks = stream.getVideoTracks();
  const ended = () => {
    if (!disposed && isCurrent()) fail(new Error("Camera disconnected"));
  };
  tracks.forEach((track) => {
    track.addEventListener("ended", ended);
  });
  const stop = () => {
    disposed = true;
    clock?.dispose();
    tracks.forEach((track) => {
      track.removeEventListener("ended", ended);
    });
    video.pause();
    video.srcObject = null;
    video.load();
  };
  let playTimeout = 0;
  try {
    await Promise.race([
      video.play(),
      new Promise<never>((_, reject) => {
        playTimeout = window.setTimeout(
          () => reject(new Error("Camera playback did not become ready")),
          10_000,
        );
      }),
    ]);
  } catch (error) {
    stop();
    throw error;
  } finally {
    window.clearTimeout(playTimeout);
  }
  if (!isCurrent()) {
    stop();
    return stop;
  }
  try {
    clock = new HuddleMediaClock((error) => {
      if (!disposed && isCurrent()) {
        stop();
        fail(error);
      }
    });
  } catch (error) {
    stop();
    throw error;
  }
  const frameClock = clock;
  let lastFrame = performance.now();
  let lastSourceFrame = -1;
  async function tick() {
    if (disposed || !isCurrent()) return;
    const before = performance.now();
    try {
      if (
        video.readyState >= 2 &&
        video.videoWidth > 0 &&
        video.videoHeight > 0
      ) {
        const decoded = video.getVideoPlaybackQuality?.().totalVideoFrames;
        const sourceFrame =
          decoded && decoded > 0 ? decoded : video.currentTime;
        if (sourceFrame === lastSourceFrame) {
          if (performance.now() - lastFrame > 5000)
            throw new Error("Camera stopped producing video");
          frameClock.schedule(() => void tick(), 1000 / CAMERA_FPS);
          return;
        }
        lastSourceFrame = sourceFrame;
        lastFrame = performance.now();
        const scale = Math.min(
          1,
          CAMERA_WIDTH / video.videoWidth,
          CAMERA_HEIGHT / video.videoHeight,
        );
        const width = Math.floor(video.videoWidth * scale) & ~1;
        const height = Math.floor(video.videoHeight * scale) & ~1;
        if (width < 16 || height < 16)
          throw new Error("Camera video is too small");
        if (canvas.width !== width || canvas.height !== height) {
          canvas.width = width;
          canvas.height = height;
        }
        context.drawImage(video, 0, 0, width, height);
        const image = context.getImageData(0, 0, width, height);
        if (disposed || !isCurrent()) return;
        await submit(new Uint8Array(image.data.buffer), width, height);
      } else if (performance.now() - lastFrame > 5000) {
        throw new Error("Camera stopped producing video");
      }
      if (!disposed && isCurrent()) {
        frameClock.schedule(
          () => void tick(),
          Math.max(0, 1000 / CAMERA_FPS - (performance.now() - before)),
        );
      }
    } catch (error) {
      if (!disposed && isCurrent()) {
        stop();
        fail(error);
      }
    }
  }
  void tick();
  return stop;
}
