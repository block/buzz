import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as React from "react";

export type VoiceInputMode = "push_to_talk" | "voice_activity";

export function useHuddlePttState(micConnected: boolean) {
  const [pttActive, setPttActive] = React.useState(false);
  const [voiceInputMode, setVoiceInputModeState] =
    React.useState<VoiceInputMode>("voice_activity");
  const voiceInputModeRef = React.useRef<VoiceInputMode>("voice_activity");
  voiceInputModeRef.current = voiceInputMode;
  const getVoiceInputMode = React.useCallback(
    () => voiceInputModeRef.current,
    [],
  );
  const pttAudioCtxRef = React.useRef<AudioContext | null>(null);

  React.useEffect(() => {
    invoke<VoiceInputMode>("get_voice_input_mode")
      .then(setVoiceInputModeState)
      .catch(() => {
        /* best-effort — default is voice_activity */
      });
  }, []);

  React.useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<boolean>("ptt-state", (event) => {
      if (cancelled) return;
      setPttActive(event.payload);
      if (!micConnected) return;
      try {
        if (
          !pttAudioCtxRef.current ||
          pttAudioCtxRef.current.state === "closed"
        ) {
          pttAudioCtxRef.current = new AudioContext();
        }
        const context = pttAudioCtxRef.current;
        const oscillator = context.createOscillator();
        const gain = context.createGain();
        oscillator.connect(gain);
        gain.connect(context.destination);
        oscillator.frequency.value = event.payload ? 880 : 440;
        gain.gain.value = 0.05;
        oscillator.start();
        oscillator.stop(context.currentTime + 0.05);
      } catch {
        /* best-effort */
      }
    }).then((cleanup) => {
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
      if (pttAudioCtxRef.current?.state !== "closed") {
        void pttAudioCtxRef.current?.close();
        pttAudioCtxRef.current = null;
      }
    };
  }, [micConnected]);

  return {
    getVoiceInputMode,
    pttActive,
    setVoiceInputModeState,
    voiceInputMode,
  };
}
