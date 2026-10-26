type MicrophoneDevices = Pick<
  MediaDevices,
  "getUserMedia" | "getSupportedConstraints"
>;

function errorInfo(error: unknown) {
  const value = error as { name?: unknown; message?: unknown } | null;
  return {
    name: typeof value?.name === "string" ? value.name : "",
    message: typeof value?.message === "string" ? value.message : "",
  };
}

function constraintFailure(error: unknown): boolean {
  const { name, message } = errorInfo(error);
  return (
    name === "OverconstrainedError" ||
    name === "ConstraintNotSatisfiedError" ||
    ((name === "Error" || name === "TypeError" || name === "") &&
      message === "Invalid constraint")
  );
}

function noInput(error: unknown): boolean {
  return errorInfo(error).name === "NotFoundError";
}

/** Acquire an optional mic without changing a user's explicitly selected device. */
export async function acquireHuddleMicrophone(
  devices: MicrophoneDevices,
  selectedDeviceId: string,
  isCurrent: () => boolean,
): Promise<MediaStream | null> {
  const supported = devices.getSupportedConstraints();
  const audio: MediaTrackConstraints = {};
  if (supported.echoCancellation) audio.echoCancellation = true;
  if (supported.noiseSuppression) audio.noiseSuppression = true;
  if (supported.sampleRate) audio.sampleRate = 48_000;
  if (selectedDeviceId) audio.deviceId = { exact: selectedDeviceId };
  const ensureCurrent = () => {
    if (!isCurrent()) throw new Error("superseded");
  };
  ensureCurrent();
  let stream: MediaStream;
  try {
    stream = await devices.getUserMedia({ audio });
  } catch (error) {
    ensureCurrent();
    if (noInput(error)) return null;
    if (!constraintFailure(error)) throw error;
    // WebKit can reject processing/rate preferences on virtual inputs. One
    // minimal retry distinguishes an unsupported preference from no usable mic.
    try {
      stream = await devices.getUserMedia({
        audio: selectedDeviceId
          ? { deviceId: { exact: selectedDeviceId } }
          : true,
      });
    } catch (fallbackError) {
      ensureCurrent();
      if (noInput(fallbackError) || constraintFailure(fallbackError))
        return null;
      throw fallbackError;
    }
  }
  if (!isCurrent() || stream.getAudioTracks().length === 0) {
    stream.getTracks().forEach((track) => {
      track.stop();
    });
    ensureCurrent();
    throw new Error("Microphone capture returned no audio track.");
  }
  return stream;
}
