import * as React from "react";

import type { AudioWorkletHandle } from "./audioWorklet";

export type AudioInputDevice = {
  deviceId: string;
  label: string;
};

/**
 * Manages audio input device enumeration, device selection, and mic gain.
 * Extracted from HuddleContext to keep file sizes manageable.
 *
 * Enumeration is strictly demand-driven: no mount-time enumeration and no
 * `devicechange` listener. In WebKitGTK every `enumerateDevices()` call starts
 * fresh GStreamer device-monitor machinery, and each monitor start
 * re-announces the existing devices as `devicechange` — so re-enumerating on
 * `devicechange` is a self-sustaining loop that leaks file descriptors in the
 * web process until it dies (see docs/linux-media-device-enumeration-loop.md).
 * The list is refreshed only when a consumer needs it: when the mic picker
 * opens (MicControls) and once after `getUserMedia` succeeds in HuddleContext
 * (device labels only become readable once capture permission is granted).
 *
 * Concurrent refreshes are serialized (a call made while one is in flight
 * awaits it instead of enumerating again) and a result identical to the
 * previous list is dropped, so a burst of triggers cannot churn React state
 * or re-trigger device monitors.
 */
export function useAudioDevices(
  workletRef: React.RefObject<AudioWorkletHandle | null>,
) {
  const [audioDevices, setAudioDevices] = React.useState<AudioInputDevice[]>(
    [],
  );
  const [selectedDeviceId, setSelectedDeviceId] = React.useState("");
  const [micGain, setMicGainState] = React.useState(1);
  const micGainRef = React.useRef(1);

  const inFlightRef = React.useRef<Promise<void> | null>(null);
  const lastListJsonRef = React.useRef<string | null>(null);

  const refreshAudioDevices = React.useCallback(async () => {
    if (inFlightRef.current) {
      return inFlightRef.current;
    }
    const run = (async () => {
      try {
        const devices = await navigator.mediaDevices.enumerateDevices();
        const next = devices
          .filter((device) => device.kind === "audioinput")
          .map((device) => ({
            deviceId: device.deviceId,
            label: device.label,
          }));
        const nextJson = JSON.stringify(next);
        if (nextJson !== lastListJsonRef.current) {
          lastListJsonRef.current = nextJson;
          setAudioDevices(next);
        }
      } catch (error) {
        // Best-effort refresh: keep the previous list; the next demand-driven
        // trigger retries. Log so a silent media failure stays debuggable.
        console.error(
          "[huddle] Failed to enumerate audio input devices:",
          error,
        );
      } finally {
        inFlightRef.current = null;
      }
    })();
    inFlightRef.current = run;
    return run;
  }, []);

  const setMicGain = React.useCallback(
    (value: number) => {
      const clamped = Math.max(0, Math.min(1, value));
      micGainRef.current = clamped;
      setMicGainState(clamped);
      workletRef.current?.setGain(clamped);
    },
    [workletRef],
  );

  return {
    audioDevices,
    refreshAudioDevices,
    selectedDeviceId,
    setSelectedDeviceId,
    micGain,
    setMicGain,
  };
}
