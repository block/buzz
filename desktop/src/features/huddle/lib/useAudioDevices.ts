import * as React from "react";

import type { AudioWorkletHandle } from "./audioWorklet";
import { availableMediaDevices } from "./mediaDevices";

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
  // No-op where `navigator.mediaDevices` is absent (non-secure context) —
  // the device list stays empty rather than crashing the tree on mount.
  React.useEffect(() => {
    const media = availableMediaDevices();
    if (!media) return;

    // Arrow const, not a hoisted `function` — a declaration would float above
    // the null guard and lose the narrowing on `media`.
    const refreshDevices = () => {
      media
        .enumerateDevices()
        .then((devices) =>
          setAudioDevices(devices.filter((d) => d.kind === "audioinput")),
        )
        .catch(() => {
          /* best-effort */
        });
    };
    refreshDevices();
    media.addEventListener("devicechange", refreshDevices);
    return () => {
      media.removeEventListener("devicechange", refreshDevices);
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
