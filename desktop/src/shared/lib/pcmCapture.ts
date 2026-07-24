/**
 * Raw binary invoke over Tauri's internal IPC.
 *
 * The public typed API does not support InvokeBody::Raw. Keeping the internal
 * dependency here gives huddles and composer dictation one upgrade boundary.
 */
function invokeRawBinary(
  command: string,
  payload: Uint8Array,
): Promise<unknown> {
  // biome-ignore lint/suspicious/noExplicitAny: Tauri internals have no public type definition
  const internals = (window as any).__TAURI_INTERNALS__;
  if (!internals?.invoke) {
    return Promise.reject(new Error("Tauri internals not available"));
  }
  return internals.invoke(command, payload);
}

export type PcmCaptureHandle = {
  stop: () => void;
  setGain: (value: number) => void;
  setTransmitting: (active: boolean) => void;
};

/**
 * Capture a microphone track as 48 kHz mono f32 PCM and forward it to a raw
 * Tauri command. The command selects the isolated Rust consumer (huddle or
 * composer dictation).
 */
export async function setupPcmCapture(
  audioTrack: MediaStreamTrack,
  command: "push_audio_pcm" | "push_dictation_audio_pcm",
  initialTransmitting = true,
): Promise<PcmCaptureHandle> {
  const audioContext = new AudioContext({ sampleRate: 48000 });
  if (audioContext.state === "suspended") {
    await audioContext.resume();
  }

  await audioContext.audioWorklet.addModule("/worklet.js");
  const source = audioContext.createMediaStreamSource(
    new MediaStream([audioTrack]),
  );
  const gainNode = audioContext.createGain();
  const workletNode = new AudioWorkletNode(audioContext, "stt-tap-processor");

  source.connect(gainNode);
  gainNode.connect(workletNode);
  if (!initialTransmitting) {
    workletNode.port.postMessage({ type: "ptt", active: false });
  }

  workletNode.port.onmessage = (event: MessageEvent<Float32Array>) => {
    const float32 = event.data;
    invokeRawBinary(
      command,
      new Uint8Array(float32.buffer, float32.byteOffset, float32.byteLength),
    ).catch(() => {
      // The Rust queues are deliberately lossy under backpressure.
    });
  };

  return {
    stop: () => {
      workletNode.port.onmessage = null;
      source.disconnect();
      gainNode.disconnect();
      workletNode.disconnect();
      void audioContext.close();
    },
    setGain: (value: number) => {
      gainNode.gain.value = value;
    },
    setTransmitting: (active: boolean) => {
      workletNode.port.postMessage({ type: "ptt", active });
    },
  };
}
