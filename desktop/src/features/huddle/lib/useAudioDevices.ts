import * as React from "react";

import type { AudioWorkletHandle } from "./audioWorklet";

export type AudioInputDevice = {
  deviceId: string;
  label: string;
};

/**
 * Manages audio input device enumeration, device selection, and mic gain.
 * Extracted from HuddleContext to keep file sizes manageable.
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

  // Enumerate audio input devices on mount and when devices change.
  React.useEffect(() => {
    const mediaDevices = navigator.mediaDevices;
    // Media capture may be unavailable in a desktop webview. Chat must still mount.
    if (!mediaDevices) return;
    function refreshDevices() {
      mediaDevices
        .enumerateDevices()
        .then((devices) =>
          setAudioDevices(
            devices
              .filter((device) => device.kind === "audioinput")
              .map((device) => ({
                deviceId: device.deviceId,
                label: device.label,
              })),
          ),
        )
        .catch(() => {
          /* best-effort */
        });
    }
    refreshDevices();
    mediaDevices.addEventListener("devicechange", refreshDevices);
    return () => {
      mediaDevices.removeEventListener("devicechange", refreshDevices);
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
