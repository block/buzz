import * as React from "react";

import type { AudioWorkletHandle } from "./audioWorklet";

/**
 * Manages audio input device enumeration, device selection, and mic gain.
 * Extracted from HuddleContext to keep file sizes manageable.
 */
export function useAudioDevices(
  workletRef: React.RefObject<AudioWorkletHandle | null>,
) {
  const [audioDevices, setAudioDevices] = React.useState<MediaDeviceInfo[]>([]);
  const [selectedDeviceId, setSelectedDeviceId] = React.useState("");
  const [micGain, setMicGainState] = React.useState(1);
  const micGainRef = React.useRef(1);

  // Enumerate audio input devices on mount and when devices change.
  React.useEffect(() => {
    // WebKitGTK's libcamera backend can retry a failing camera manager in a
    // tight loop, firing `devicechange` on every cycle. Each cycle spawns a new
    // libcamera manager and leaks file descriptors in the web process; on
    // machines where the camera fails to enumerate ("No such device") this
    // escalates until EMFILE aborts the app ("Too many open files", GLib
    // "Creating pipes for GWakeup"). Throttle refreshes so the frontend can't
    // amplify the loop, and back off after repeated failures.
    const MIN_REFRESH_MS = 2000;
    const BACKOFF_MS = 30_000;
    const MAX_FAILURES = 3;
    let lastRefresh = 0;
    let failures = 0;
    let backoffUntil = 0;

    function refreshDevices() {
      const now = Date.now();
      if (now < backoffUntil || now - lastRefresh < MIN_REFRESH_MS) {
        return;
      }
      lastRefresh = now;
      navigator.mediaDevices
        .enumerateDevices()
        .then((devices) => {
          failures = 0;
          setAudioDevices(devices.filter((d) => d.kind === "audioinput"));
        })
        .catch(() => {
          failures += 1;
          if (failures >= MAX_FAILURES) {
            backoffUntil = Date.now() + BACKOFF_MS;
            failures = 0;
          }
        });
    }
    refreshDevices();
    navigator.mediaDevices.addEventListener("devicechange", refreshDevices);
    return () => {
      navigator.mediaDevices.removeEventListener(
        "devicechange",
        refreshDevices,
      );
    };
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
    selectedDeviceId,
    setSelectedDeviceId,
    micGain,
    setMicGain,
  };
}
