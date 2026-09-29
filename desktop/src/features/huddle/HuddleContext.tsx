import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import * as React from "react";

import { useHuddleAudioReconnect } from "./hooks/useHuddleAudioReconnect";
import { setupAudioWorklet, type AudioWorkletHandle } from "./lib/audioWorklet";
import {
  HUDDLE_AUDIO_COMMAND_EVENT,
  HUDDLE_AUDIO_LEVEL_EVENT,
  HUDDLE_AUDIO_STATE_EVENT,
  type HuddleAudioCommand,
  type HuddleAudioMirrorState,
  interruptAgentSpeech,
  isRedundantHuddlePhaseError,
} from "./lib/huddleAudioBridge";
import {
  shouldClaimHuddleMedia,
  shouldEndRustSessionOnAudioOwnerUnmount,
  shouldSetupMediaOnHuddleStart,
} from "./lib/huddleAudioSession";
import { useAudioDevices } from "./lib/useAudioDevices";
import { usePipelineHotstart } from "./lib/usePipelineHotstart";
import { formatHuddleActionError } from "./lib/huddleError";
import {
  type VoiceInputMode,
  useHuddlePttState,
} from "./lib/useHuddlePttState";
import { useHuddleSpeakerActivity } from "./lib/useHuddleSpeakerActivity";
import { useMicLevelAnalyser } from "./lib/useMicLevelAnalyser";
import { useTtsSubscription } from "./lib/useTtsSubscription";
import type {
  HuddleContextValue,
  HuddleLevelsValue,
} from "./HuddleContext.types";

/**
 * Huddle lifecycle (React context):
 *   Main (ownsAudioSession=false): startHuddle/joinHuddle → invoke(start/join_huddle)
 *     → open companion (no getUserMedia; do not confirm Active yet).
 *   Companion (ownsAudioSession=true): claim getUserMedia + AudioWorklet when Rust
 *     reports connected/active → confirm_huddle_active so PCM survives companion
 *     focus (main AudioContext is suspended by WKWebView → deaf STT).
 *   TTS subscription: only on the audio owner (companion) → speak_agent_message
 *   Audio-owner unmount: release mic only — never leave_huddle (reopen / remount).
 *   leaveHuddle: stop worklet → stop mic track → invoke(leave_huddle)
 *   Active speakers: Tauri "huddle-active-speakers" event (Rust backend emits)
 */

type HuddleJoinInfo = {
  ephemeral_channel_id: string;
};

const HuddleContext = React.createContext<HuddleContextValue | null>(null);
const HuddleLevelsContext = React.createContext<HuddleLevelsValue | null>(null);

export function HuddleProvider({
  children,
  ownsAudioSession = true,
  onHuddleStartPendingChange,
  onHuddleStarted,
  onShowHuddleInMainApp,
  onViewHuddleChannel,
}: {
  children: React.ReactNode;
  /** When true, this webview owns getUserMedia + AudioWorklet (companion room). */
  ownsAudioSession?: boolean;
  /** Keeps the main-app drawer suppressed while a new huddle is handed to its companion window. */
  onHuddleStartPendingChange?: (pending: boolean) => void;
  /** Called after a huddle has connected its local audio. */
  onHuddleStarted?: (ephemeralChannelId: string) => void | Promise<void>;
  /** Reveals a huddle's temporary channel and navigates the main app to it. */
  onShowHuddleInMainApp?: (ephemeralChannelId: string) => void;
  /** Reveals an active or archived Huddle channel in the main app. */
  onViewHuddleChannel?: (ephemeralChannelId: string) => void;
}) {
  const workletRef = React.useRef<AudioWorkletHandle | null>(null);
  const tokenRef = React.useRef(0);
  const busyRef = React.useRef(false);
  /** True once Rust `start_huddle` or `join_huddle` has been invoked (even if JS-side refs aren't populated yet). */
  const rustActiveRef = React.useRef(false);
  const [localAudioTrack, setLocalAudioTrack] =
    React.useState<MediaStreamTrack | null>(null);
  const [isStarting, setIsStarting] = React.useState(false);
  const [huddleError, setHuddleError] = React.useState<string | null>(null);
  const clearHuddleError = React.useCallback(() => setHuddleError(null), []);
  const [micConnected, setMicConnected] = React.useState(false);
  const [isMuted, setIsMuted] = React.useState(false);
  const isMutedRef = React.useRef(isMuted);
  isMutedRef.current = isMuted;
  const micConnectedRef = React.useRef(micConnected);
  micConnectedRef.current = micConnected;
  const [mirroredAudioState, setMirroredAudioState] =
    React.useState<HuddleAudioMirrorState | null>(null);
  const [mirroredMicLevel, setMirroredMicLevel] = React.useState(0);
  const {
    getVoiceInputMode,
    pttActive,
    setVoiceInputModeState,
    voiceInputMode,
  } = useHuddlePttState(micConnected);
  // Manual mute remains independently controllable in every input mode. The
  // PTT shortcut temporarily opens a manually muted microphone while held.
  const locallyMuted =
    isMuted && !(voiceInputMode === "push_to_talk" && pttActive);
  const locallyMutedRef = React.useRef(locallyMuted);
  locallyMutedRef.current = locallyMuted;
  /** Ephemeral channel ID — set after start_huddle/join_huddle, used for TTS subscription */
  const [ephemeralChannelId, setEphemeralChannelId] = React.useState<
    string | null
  >(null);
  /** Self pubkey — fetched once, used to filter out own messages from TTS */
  const selfPubkeyRef = React.useRef<string | null>(null);
  const { activeSpeakers, resetSpeakerActivity, speakerLevels } =
    useHuddleSpeakerActivity();
  const {
    audioDevices: localAudioDevices,
    selectedDeviceId: localSelectedDeviceId,
    setSelectedDeviceId: setLocalSelectedDeviceId,
    micGain: localMicGain,
    setMicGain: setLocalMicGain,
  } = useAudioDevices(workletRef);
  const audioDevices = ownsAudioSession
    ? localAudioDevices
    : (mirroredAudioState?.audioDevices ?? []);
  const selectedDeviceId = ownsAudioSession
    ? localSelectedDeviceId
    : (mirroredAudioState?.selectedDeviceId ?? "");
  const micGain = ownsAudioSession
    ? localMicGain
    : (mirroredAudioState?.micGain ?? 1);
  const effectiveVoiceInputMode = ownsAudioSession
    ? voiceInputMode
    : (mirroredAudioState?.voiceInputMode ?? voiceInputMode);
  const effectiveIsMuted = ownsAudioSession
    ? locallyMuted
    : (mirroredAudioState?.isMuted ?? true);
  const setSelectedDeviceId = React.useCallback(
    (deviceId: string) => {
      if (ownsAudioSession) {
        setLocalSelectedDeviceId(deviceId);
        return;
      }
      setMirroredAudioState((previous) =>
        previous ? { ...previous, selectedDeviceId: deviceId } : previous,
      );
      void emit(HUDDLE_AUDIO_COMMAND_EVENT, {
        type: "set-input-device",
        deviceId,
      } satisfies HuddleAudioCommand);
    },
    [ownsAudioSession, setLocalSelectedDeviceId],
  );
  const setMicGain = React.useCallback(
    (gain: number) => {
      const clamped = Math.max(0, Math.min(1, gain));
      if (ownsAudioSession) {
        setLocalMicGain(clamped);
        return;
      }
      setMirroredAudioState((previous) =>
        previous ? { ...previous, micGain: clamped } : previous,
      );
      void emit(HUDDLE_AUDIO_COMMAND_EVENT, {
        type: "set-mic-gain",
        gain: clamped,
      } satisfies HuddleAudioCommand);
    },
    [ownsAudioSession, setLocalMicGain],
  );
  /** Audio output devices from Rust backend */
  const [outputDevices, setOutputDevices] = React.useState<
    { name: string; is_default: boolean }[]
  >([]);
  const [selectedOutputDevice, setSelectedOutputDeviceState] =
    React.useState("");
  const setSelectedOutputDevice = React.useCallback((name: string) => {
    setSelectedOutputDeviceState(name);
    invoke("set_audio_output_device", { name }).catch(() => {
      /* best-effort */
    });
  }, []);

  // Fetch output devices on mount and when system devices change.
  React.useEffect(() => {
    function refreshOutputDevices() {
      invoke<{ name: string; is_default: boolean }[]>(
        "list_audio_output_devices",
      )
        .then(setOutputDevices)
        .catch(() => {
          /* best-effort */
        });
    }
    refreshOutputDevices();
    invoke<string>("get_audio_output_device")
      .then(setSelectedOutputDeviceState)
      .catch(() => {
        /* best-effort */
      });
    navigator.mediaDevices.addEventListener(
      "devicechange",
      refreshOutputDevices,
    );
    return () => {
      navigator.mediaDevices.removeEventListener(
        "devicechange",
        refreshOutputDevices,
      );
    };
  }, []);

  /** Ref tracking latest micGain — read inside connectAndSetupMedia to
   *  avoid stale closure capture. */
  const micGainRef = React.useRef(1);
  micGainRef.current = micGain;

  // Toggle voice input mode — persists to Rust backend and updates worklet gating.
  // Entering Push to Talk always starts gated (hold key to transmit). Non-owners
  // must not read the unused local isMutedRef (stays false) or they reopen the
  // Rust STT / relay gate and leave the companion mic continuously hot.
  const setVoiceInputMode = React.useCallback(
    async (mode: VoiceInputMode) => {
      await invoke("set_voice_input_mode", { mode });
      setVoiceInputModeState(mode);
      const enteringPtt = mode === "push_to_talk";

      if (ownsAudioSession) {
        if (enteringPtt) {
          isMutedRef.current = true;
          setIsMuted(true);
          void invoke("set_huddle_manual_mic_unmuted", {
            enabled: false,
          }).catch(() => {});
          workletRef.current?.setMode(mode);
          workletRef.current?.setTransmitting(false);
        } else {
          void invoke("set_huddle_manual_mic_unmuted", {
            enabled: !isMutedRef.current,
          }).catch(() => {});
          workletRef.current?.setMode(mode);
        }
        return;
      }

      if (enteringPtt) {
        void invoke("set_huddle_manual_mic_unmuted", {
          enabled: false,
        }).catch(() => {});
        setMirroredAudioState((previous) =>
          previous
            ? { ...previous, isMuted: true, voiceInputMode: mode }
            : previous,
        );
        void emit(HUDDLE_AUDIO_COMMAND_EVENT, {
          type: "set-voice-input-mode",
          mode,
        } satisfies HuddleAudioCommand);
        void emit(HUDDLE_AUDIO_COMMAND_EVENT, {
          type: "set-muted",
          isMuted: true,
        } satisfies HuddleAudioCommand);
        return;
      }

      setMirroredAudioState((previous) => {
        const muted = previous?.isMuted ?? true;
        void invoke("set_huddle_manual_mic_unmuted", {
          enabled: !muted,
        }).catch(() => {});
        return previous ? { ...previous, voiceInputMode: mode } : previous;
      });
      void emit(HUDDLE_AUDIO_COMMAND_EVENT, {
        type: "set-voice-input-mode",
        mode,
      } satisfies HuddleAudioCommand);
    },
    [ownsAudioSession, setVoiceInputModeState],
  );

  // Keep disconnectMedia stable so setting the track cannot re-fire the
  // unmount cleanup during startup.
  const audioTrackRef = React.useRef<MediaStreamTrack | null>(null);
  audioTrackRef.current = localAudioTrack;

  // Keep the browser track and worklet aligned with the combined manual/PTT
  // state. The worklet tracks the manual state separately so a PTT release
  // does not remute a microphone the user explicitly left open.
  React.useEffect(() => {
    if (!ownsAudioSession || !audioTrackRef.current) return;
    audioTrackRef.current.enabled = !locallyMuted;
    workletRef.current?.setTransmitting(!isMuted);
  }, [isMuted, locallyMuted, ownsAudioSession]);

  const toggleMute = React.useCallback(() => {
    if (!ownsAudioSession) {
      const nextMuted = !(mirroredAudioState?.isMuted ?? false);
      setMirroredAudioState((previous) => ({
        isMuted: nextMuted,
        micConnected: previous?.micConnected ?? false,
        audioDevices: previous?.audioDevices ?? [],
        selectedDeviceId: previous?.selectedDeviceId ?? "",
        micGain: previous?.micGain ?? 1,
        voiceInputMode: previous?.voiceInputMode ?? voiceInputMode,
      }));
      // The companion never owns a MediaStream. Send the intended state, not
      // a toggle, so a delayed initial state response cannot invert the main
      // window's live microphone track.
      void emit(HUDDLE_AUDIO_COMMAND_EVENT, {
        type: "set-muted",
        isMuted: nextMuted,
      });
      return;
    }

    // Set the effective state promised by the button instead of inverting the
    // hidden manual preference, which can differ while PTT is held.
    const requestedMuted = !locallyMuted;
    setIsMuted(requestedMuted);
    void invoke("set_huddle_manual_mic_unmuted", {
      enabled: !requestedMuted,
    });
  }, [
    locallyMuted,
    mirroredAudioState?.isMuted,
    ownsAudioSession,
    voiceInputMode,
  ]);

  React.useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let requestRetry: number | null = null;

    listen<HuddleAudioMirrorState>(HUDDLE_AUDIO_STATE_EVENT, (event) => {
      if (!cancelled && !ownsAudioSession) {
        if (requestRetry !== null) {
          window.clearInterval(requestRetry);
          requestRetry = null;
        }
        setMirroredAudioState(event.payload);
        setVoiceInputModeState(event.payload.voiceInputMode);
      }
    }).then((fn) => {
      if (cancelled) {
        fn();
        return;
      }
      unlisten = fn;

      if (!ownsAudioSession) {
        // Register the response listener before asking the main window for its
        // browser-owned microphone state. The prior fire-and-forget request
        // could be answered before this listener existed, leaving the room
        // window permanently stuck in its "microphone unavailable" fallback.
        const requestState = () => {
          void emit(HUDDLE_AUDIO_COMMAND_EVENT, {
            type: "request-state",
          } satisfies HuddleAudioCommand);
        };
        requestState();
        // A brief retry also covers the main window rebuilding its listener
        // during a device-change render. It stops with the first response.
        requestRetry = window.setInterval(requestState, 500);
      }
    });

    return () => {
      cancelled = true;
      if (requestRetry !== null) window.clearInterval(requestRetry);
      unlisten?.();
    };
  }, [ownsAudioSession, setVoiceInputModeState]);

  React.useEffect(() => {
    if (!ownsAudioSession) return;

    const state: HuddleAudioMirrorState = {
      isMuted: locallyMuted,
      micConnected,
      audioDevices: localAudioDevices,
      selectedDeviceId: localSelectedDeviceId,
      micGain: localMicGain,
      voiceInputMode,
    };
    void emit(HUDDLE_AUDIO_STATE_EVENT, state);

    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<HuddleAudioCommand>(HUDDLE_AUDIO_COMMAND_EVENT, (event) => {
      if (cancelled) return;
      if (event.payload.type === "set-muted") {
        const requestedMuted = event.payload.isMuted;
        setIsMuted(() => {
          void invoke("set_huddle_manual_mic_unmuted", {
            enabled: !requestedMuted,
          });
          return requestedMuted;
        });
        return;
      }
      if (event.payload.type === "set-input-device") {
        setLocalSelectedDeviceId(event.payload.deviceId);
        return;
      }
      if (event.payload.type === "set-mic-gain") {
        setLocalMicGain(event.payload.gain);
        return;
      }
      if (event.payload.type === "set-voice-input-mode") {
        const nextMode = event.payload.mode;
        setVoiceInputModeState(nextMode);
        workletRef.current?.setMode(nextMode);
        // Defense in depth: companion capture must gate when PTT is enabled
        // from the main window, even if set-muted races behind this command.
        if (nextMode === "push_to_talk") {
          isMutedRef.current = true;
          setIsMuted(true);
          workletRef.current?.setTransmitting(false);
          void invoke("set_huddle_manual_mic_unmuted", {
            enabled: false,
          }).catch(() => {});
        }
        return;
      }
      void emit(HUDDLE_AUDIO_STATE_EVENT, {
        isMuted: locallyMutedRef.current,
        micConnected: micConnectedRef.current,
        audioDevices: localAudioDevices,
        selectedDeviceId: localSelectedDeviceId,
        micGain: localMicGain,
        voiceInputMode,
      } satisfies HuddleAudioMirrorState);
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [
    locallyMuted,
    localAudioDevices,
    localMicGain,
    localSelectedDeviceId,
    micConnected,
    ownsAudioSession,
    setLocalMicGain,
    setLocalSelectedDeviceId,
    setVoiceInputModeState,
    voiceInputMode,
  ]);

  /** Stop AudioWorklet and mic track. Best-effort on all steps. */
  const disconnectMedia = React.useCallback(async () => {
    // Invalidate any in-flight startHuddle/joinHuddle
    tokenRef.current += 1;
    try {
      workletRef.current?.stop();
    } catch {
      /* best-effort */
    }
    workletRef.current = null;
    audioTrackRef.current?.stop();
    setLocalAudioTrack(null);
    setMicConnected(false);
    setEphemeralChannelId(null);
    resetSpeakerActivity();
  }, [resetSpeakerActivity]); // Stable — reads track from ref, not state.

  // Keep ephemeral channel + capture keyed to Rust across remounts.
  // Audio owner releases mic on Idle. Non-owner (main) still tracks the
  // ephemeral id for leave shortcuts while companion owns capture.
  React.useEffect(() => {
    type HuddleBackendState = {
      phase?: string;
      ephemeral_channel_id?: string | null;
    };

    const applyBackendState = (state: HuddleBackendState) => {
      if (state.phase === "idle") {
        if (ownsAudioSession) {
          void disconnectMedia();
        } else {
          setEphemeralChannelId(null);
        }
        return;
      }
      if (state.ephemeral_channel_id) {
        setEphemeralChannelId(state.ephemeral_channel_id);
      }
    };

    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void invoke<HuddleBackendState>("get_huddle_state")
      .then((state) => {
        if (!cancelled && state) applyBackendState(state);
      })
      .catch(() => {
        /* best-effort; lifecycle events remain authoritative */
      });
    void listen<HuddleBackendState>("huddle-state-changed", (event) => {
      if (!cancelled) applyBackendState(event.payload);
    }).then((cleanup) => {
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [disconnectMedia, ownsAudioSession]);

  const leaveHuddle = React.useCallback(async (): Promise<boolean> => {
    await disconnectMedia();
    try {
      // `leave_huddle` is idempotent in Rust. Always call it so a provider
      // remount cannot leave Rust's huddle state active while this ref is false.
      await invoke("leave_huddle");
      rustActiveRef.current = false;
    } catch {
      return false; // Signal that backend cleanup failed
    }
    return true; // Backend cleanup succeeded (or was not needed)
  }, [disconnectMedia]);

  /**
   * Clean up a partially-established huddle. Best-effort on every step.
   *
   * Takes explicit worklet/stream args (not from refs) because startHuddle/joinHuddle
   * may have local variables that differ from the refs mid-flight.
   */
  const cleanupFailedStart = React.useCallback(
    async (worklet: AudioWorkletHandle | null, isCreator: boolean) => {
      try {
        worklet?.stop();
      } catch {
        /* best-effort */
      }
      setLocalAudioTrack(null);
      setMicConnected(false);
      setEphemeralChannelId(null);
      resetSpeakerActivity();
      if (rustActiveRef.current) {
        if (isCreator) {
          try {
            await invoke("end_huddle");
            rustActiveRef.current = false;
          } catch {
            try {
              await invoke("leave_huddle");
              rustActiveRef.current = false;
            } catch {}
          }
        } else {
          try {
            await invoke("leave_huddle");
            rustActiveRef.current = false;
          } catch {}
        }
      }
    },
    [resetSpeakerActivity],
  );

  /**
   * Clean up only this provider's media after its start token is superseded.
   * The action that changed the token owns backend teardown; issuing a global
   * leave here could terminate a replacement huddle started by a new provider.
   */
  const cleanupSupersededStart = React.useCallback(
    (worklet: AudioWorkletHandle | null) => {
      try {
        worklet?.stop();
      } catch {
        /* best-effort */
      }
      workletRef.current = null;
      rustActiveRef.current = false;
      setLocalAudioTrack(null);
      setMicConnected(false);
      setEphemeralChannelId(null);
      resetSpeakerActivity();
    },
    [resetSpeakerActivity],
  );

  /** Shared media setup: get mic, setup AudioWorklet, confirm active.
   *  Used by both startHuddle and joinHuddle after the Rust backend call succeeds. */
  const connectAndSetupMedia = React.useCallback(
    async (
      joinInfo: HuddleJoinInfo,
      myToken: number,
      options?: {
        mode?: VoiceInputMode;
        manuallyUnmuted?: boolean;
      },
    ): Promise<{
      worklet: AudioWorkletHandle;
      stream: MediaStream;
    }> => {
      // Fetch self pubkey once for TTS filtering
      if (!selfPubkeyRef.current) {
        try {
          const identity = await invoke<{ pubkey: string }>("get_identity");
          selfPubkeyRef.current = identity.pubkey;
        } catch {
          /* best-effort */
        }
      }

      if (tokenRef.current !== myToken) throw new Error("superseded");

      // Get mic — Rust backend owns the audio WS connection.
      // Request 48 kHz to match the Opus encoder and worklet buffer size (960 samples = 20ms).
      const audioConstraints: MediaTrackConstraints = {
        echoCancellation: true,
        noiseSuppression: true,
        sampleRate: 48000,
      };
      if (selectedDeviceId) {
        audioConstraints.deviceId = { exact: selectedDeviceId };
      }
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: audioConstraints,
      });
      const audioTrack = stream.getAudioTracks()[0];

      // Wrap post-getUserMedia steps so the stream is always cleaned up on
      // failure — prevents the mic permission light staying on after errors.
      try {
        if (tokenRef.current !== myToken) {
          throw new Error("superseded");
        }

        setLocalAudioTrack(audioTrack);
        setMicConnected(true);

        // Setup AudioWorklet — PCM goes to Rust via push_audio_pcm
        audioTrack.enabled = !locallyMutedRef.current;
        const mode = options?.mode ?? getVoiceInputMode();
        const manuallyUnmuted = options?.manuallyUnmuted ?? !isMutedRef.current;
        const worklet = await setupAudioWorklet(
          audioTrack,
          mode,
          manuallyUnmuted,
        );
        worklet.setGain(micGainRef.current);

        if (tokenRef.current !== myToken) {
          worklet.stop();
          throw new Error("superseded");
        }

        workletRef.current = worklet;
        setEphemeralChannelId(joinInfo.ephemeral_channel_id);
        await invoke("confirm_huddle_active");

        return { worklet, stream };
      } catch (err) {
        // Always stop the mic stream on any failure path.
        stream.getTracks().forEach((t) => {
          t.stop();
        });
        setLocalAudioTrack(null);
        setMicConnected(false);
        throw err;
      }
    },
    [getVoiceInputMode, selectedDeviceId],
  );

  const startHuddle = React.useCallback(
    async (
      parentChannelId: string,
      memberPubkeys: string[],
      channelName?: string,
    ) => {
      if (busyRef.current) return;
      busyRef.current = true;

      tokenRef.current += 1;
      const myToken = tokenRef.current;

      // PTT starts muted (must match the Rust manual_mic_unmuted default).
      const startMuted = getVoiceInputMode() === "push_to_talk";
      isMutedRef.current = startMuted;
      setIsMuted(startMuted);
      setHuddleError(null);
      setIsStarting(true);
      onHuddleStartPendingChange?.(true);
      try {
        const joinInfo = await invoke<HuddleJoinInfo>("start_huddle", {
          parentChannelId,
          memberPubkeys,
          channelName,
        });
        rustActiveRef.current = true;
        if (shouldSetupMediaOnHuddleStart(ownsAudioSession)) {
          try {
            await connectAndSetupMedia(joinInfo, myToken);
          } catch (e) {
            if (e instanceof Error && e.message === "superseded") {
              cleanupSupersededStart(workletRef.current);
              return;
            }
            throw e;
          }
        } else {
          // Companion owns capture + confirms Active after mic claim. Main must
          // not confirm here: Creating-phase companion open can remount/leave and
          // race this into `cannot confirm active: phase is Idle`.
          setEphemeralChannelId(joinInfo.ephemeral_channel_id);
        }
        try {
          await onHuddleStarted?.(joinInfo.ephemeral_channel_id);
        } catch (error) {
          // Opening the companion is presentation-only. Keep the connected
          // huddle alive if its window cannot be opened.
          console.error("Failed to present newly started huddle:", error);
        }
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        if (isRedundantHuddlePhaseError(msg)) {
          setHuddleError(null);
          // Already in this huddle — re-present UI (companion may have been
          // closed while presentation stayed on "window" with a stale open).
          try {
            const state = await invoke<{
              ephemeral_channel_id: string | null;
            }>("get_huddle_state");
            if (state.ephemeral_channel_id) {
              await onHuddleStarted?.(state.ephemeral_channel_id);
            }
          } catch (presentError) {
            console.error("Failed to re-present active huddle:", presentError);
          }
          return;
        }

        const w = workletRef.current;
        workletRef.current = null;
        await cleanupFailedStart(w, true);
        setHuddleError(formatHuddleActionError(e, "start"));
        console.error("Failed to start huddle:", e);
        throw e;
      } finally {
        onHuddleStartPendingChange?.(false);
        setIsStarting(false);
        busyRef.current = false;
      }
    },
    [
      cleanupFailedStart,
      cleanupSupersededStart,
      connectAndSetupMedia,
      getVoiceInputMode,
      onHuddleStartPendingChange,
      onHuddleStarted,
      ownsAudioSession,
    ],
  );

  const showHuddleInMainApp = React.useCallback(
    (channelId: string) => onShowHuddleInMainApp?.(channelId),
    [onShowHuddleInMainApp],
  );
  const viewHuddleChannel = React.useCallback(
    (channelId: string) => onViewHuddleChannel?.(channelId),
    [onViewHuddleChannel],
  );

  const joinHuddle = React.useCallback(
    async (
      parentChannelId: string,
      ephemeralChannelId: string,
      huddleThreadEventId?: string,
    ) => {
      if (busyRef.current) return;
      busyRef.current = true;
      tokenRef.current += 1;
      const myToken = tokenRef.current;
      const startMuted = getVoiceInputMode() === "push_to_talk";
      isMutedRef.current = startMuted;
      setIsMuted(startMuted);
      setHuddleError(null);
      setIsStarting(true);
      onHuddleStartPendingChange?.(true);

      try {
        const joinInfo = await invoke<HuddleJoinInfo>("join_huddle", {
          parentChannelId,
          ephemeralChannelId,
          huddleThreadEventId,
        });
        rustActiveRef.current = true;

        if (shouldSetupMediaOnHuddleStart(ownsAudioSession)) {
          try {
            await connectAndSetupMedia(joinInfo, myToken);
          } catch (e) {
            if (e instanceof Error && e.message === "superseded") {
              cleanupSupersededStart(workletRef.current);
              return;
            }
            throw e;
          }
        } else {
          // Same as start: companion confirms Active after claiming mic.
          setEphemeralChannelId(joinInfo.ephemeral_channel_id);
        }
        try {
          await onHuddleStarted?.(joinInfo.ephemeral_channel_id);
        } catch (error) {
          // Presentation failure must not disconnect a successfully joined
          // huddle; the user can still open its companion from the main app.
          console.error("Failed to present joined huddle:", error);
        }
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        if (isRedundantHuddlePhaseError(msg)) {
          setHuddleError(null);
          // Already joined — reopen companion / drawer instead of silent no-op.
          try {
            const state = await invoke<{
              ephemeral_channel_id: string | null;
            }>("get_huddle_state");
            if (state.ephemeral_channel_id) {
              await onHuddleStarted?.(state.ephemeral_channel_id);
            }
          } catch (presentError) {
            console.error("Failed to re-present active huddle:", presentError);
          }
          return;
        }

        const w = workletRef.current;
        workletRef.current = null;
        await cleanupFailedStart(w, false);
        setHuddleError(formatHuddleActionError(e, "join"));
        console.error("Failed to join huddle:", e);
        throw e;
      } finally {
        onHuddleStartPendingChange?.(false);
        setIsStarting(false);
        busyRef.current = false;
      }
    },
    [
      cleanupFailedStart,
      cleanupSupersededStart,
      connectAndSetupMedia,
      getVoiceInputMode,
      onHuddleStartPendingChange,
      onHuddleStarted,
      ownsAudioSession,
    ],
  );

  // Companion (audio owner) owns the one TTS subscription. Main must not
  // enqueue the same reply — it no longer captures PCM under companion focus.
  useTtsSubscription(
    ownsAudioSession ? ephemeralChannelId : null,
    selfPubkeyRef,
  );

  // Companion claims mic/worklet when Rust is already Connected/Active (main
  // start/join skipped getUserMedia). Re-claim after companion reopen.
  React.useEffect(() => {
    if (!ownsAudioSession) return;

    type HuddleBackendState = {
      phase?: string;
      ephemeral_channel_id?: string | null;
      voice_input_mode?: VoiceInputMode;
      transcription_enabled?: boolean;
    };

    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let claimInFlight = false;

    const claimMediaIfNeeded = async (state: HuddleBackendState) => {
      const ephemeral = state.ephemeral_channel_id ?? null;
      if (
        !shouldClaimHuddleMedia({
          ownsAudioSession: true,
          phase: state.phase,
          ephemeralChannelId: ephemeral,
          alreadyConnected:
            Boolean(workletRef.current) || micConnectedRef.current,
          claimInFlight,
        })
      ) {
        return;
      }
      claimInFlight = true;
      tokenRef.current += 1;
      const myToken = tokenRef.current;
      const mode = state.voice_input_mode ?? getVoiceInputMode();
      setVoiceInputModeState(mode);
      // PTT always starts gated. Agent auto-transcription flips mode to VAD in
      // Rust before claiming; do not treat transcription_enabled as a reason to
      // leave PTT continuously hot.
      const startMuted = mode === "push_to_talk";
      isMutedRef.current = startMuted;
      setIsMuted(startMuted);
      void invoke("set_huddle_manual_mic_unmuted", {
        enabled: !startMuted,
      }).catch(() => {});
      try {
        await connectAndSetupMedia(
          { ephemeral_channel_id: ephemeral as string },
          myToken,
          { mode, manuallyUnmuted: !startMuted },
        );
        rustActiveRef.current = true;
      } catch (e) {
        if (e instanceof Error && e.message === "superseded") {
          cleanupSupersededStart(workletRef.current);
          return;
        }
        console.error("Failed to claim huddle mic in companion:", e);
      } finally {
        claimInFlight = false;
      }
    };

    void invoke<HuddleBackendState>("get_huddle_state")
      .then((state) => {
        if (!cancelled && state) void claimMediaIfNeeded(state);
      })
      .catch(() => {});
    void listen<HuddleBackendState>("huddle-state-changed", (event) => {
      if (!cancelled) void claimMediaIfNeeded(event.payload);
    }).then((cleanup) => {
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [
    cleanupSupersededStart,
    connectAndSetupMedia,
    getVoiceInputMode,
    ownsAudioSession,
    setVoiceInputModeState,
  ]);

  usePipelineHotstart(ephemeralChannelId);

  // When Rust auto-enables transcription for agents it may flip voice mode to
  // VAD and open the mic. Mirror that into the worklet so PCM actually reaches
  // push_audio_pcm / STT (default PTT+muted would starve the pipeline).
  React.useEffect(() => {
    if (!ownsAudioSession) return;
    type SyncState = {
      transcription_enabled?: boolean;
      voice_input_mode?: VoiceInputMode;
      phase?: string;
    };
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    const apply = (state: SyncState) => {
      if (cancelled) return;
      if (state.phase === "idle") return;
      const mode = state.voice_input_mode;
      if (!mode) return;
      if (mode === getVoiceInputMode()) return;
      setVoiceInputModeState(mode);
      workletRef.current?.setMode(mode);
      // Backend flipped us (agent auto-enable → VAD): open the mic so STT
      // receives PCM. Do not fight a later manual mute.
      if (state.transcription_enabled && mode === "voice_activity") {
        setIsMuted(false);
        workletRef.current?.setTransmitting(true);
        void invoke("set_huddle_manual_mic_unmuted", { enabled: true }).catch(
          () => {},
        );
      }
    };
    void invoke<SyncState>("get_huddle_state")
      .then((s) => {
        if (s) apply(s);
      })
      .catch(() => {});
    void listen<SyncState>("huddle-state-changed", (event) => {
      apply(event.payload);
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [getVoiceInputMode, ownsAudioSession, setVoiceInputModeState]);

  // Mic level analyser — drives the voice activity indicator
  const micLevel = useMicLevelAnalyser(localAudioTrack, micConnected);

  React.useEffect(() => {
    if (ownsAudioSession) {
      void emit(HUDDLE_AUDIO_LEVEL_EVENT, micLevel);
    }
  }, [micLevel, ownsAudioSession]);

  React.useEffect(() => {
    if (ownsAudioSession) return;
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void listen<number>(HUDDLE_AUDIO_LEVEL_EVENT, (event) => {
      if (!cancelled) setMirroredMicLevel(event.payload);
    }).then((cleanup) => {
      if (cancelled) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [ownsAudioSession]);

  // Stable refs for async/unmount paths (avoid re-firing mid-startup).
  const leaveHuddleRef = React.useRef(leaveHuddle);
  leaveHuddleRef.current = leaveHuddle;
  const disconnectMediaRef = React.useRef(disconnectMedia);
  disconnectMediaRef.current = disconnectMedia;
  // Audio-owner unmount releases mic only. Ending the Rust session here races
  // Creating-phase companion remount/recreate against main start/join and was
  // the `cannot confirm active: phase is Idle` crash after companion-owns-audio.
  React.useEffect(() => {
    if (!ownsAudioSession) return;
    return () => {
      if (shouldEndRustSessionOnAudioOwnerUnmount()) {
        void leaveHuddleRef.current();
        return;
      }
      void disconnectMediaRef.current();
    };
  }, [ownsAudioSession]);

  useHuddleAudioReconnect({
    ownsAudioSession,
    tokenRef,
    leaveHuddleRef,
  });

  // High-frequency (20-30 Hz) audio levels live in their own context so their
  // churn re-renders only the meter components, not every useHuddle consumer.
  const levelsValue = React.useMemo<HuddleLevelsValue>(
    () => ({
      micLevel: ownsAudioSession ? micLevel : mirroredMicLevel,
      activeSpeakers,
      speakerLevels,
    }),
    [
      activeSpeakers,
      micLevel,
      mirroredMicLevel,
      ownsAudioSession,
      speakerLevels,
    ],
  );

  const effectiveMicConnected = ownsAudioSession
    ? micConnected
    : (mirroredAudioState?.micConnected ?? false);
  const contextValue = React.useMemo<HuddleContextValue>(
    () => ({
      localAudioTrack,
      isStarting,
      huddleError,
      clearHuddleError,
      micConnected: effectiveMicConnected,
      isMuted: effectiveIsMuted,
      toggleMute,
      interruptAgentSpeech,
      pttActive,
      voiceInputMode: effectiveVoiceInputMode,
      setVoiceInputMode,
      audioDevices,
      selectedDeviceId,
      setSelectedDeviceId,
      micGain,
      setMicGain,
      outputDevices,
      selectedOutputDevice,
      setSelectedOutputDevice,
      activeEphemeralChannelId: ephemeralChannelId,
      showHuddleInMainApp,
      viewHuddleChannel,
      startHuddle,
      joinHuddle,
      leaveHuddle,
    }),
    [
      audioDevices,
      clearHuddleError,
      effectiveIsMuted,
      effectiveMicConnected,
      effectiveVoiceInputMode,
      ephemeralChannelId,
      huddleError,
      isStarting,
      joinHuddle,
      leaveHuddle,
      localAudioTrack,
      micGain,
      outputDevices,
      pttActive,
      selectedDeviceId,
      selectedOutputDevice,
      setMicGain,
      setSelectedDeviceId,
      setSelectedOutputDevice,
      setVoiceInputMode,
      showHuddleInMainApp,
      startHuddle,
      toggleMute,
      viewHuddleChannel,
    ],
  );

  return (
    <HuddleContext.Provider value={contextValue}>
      <HuddleLevelsContext.Provider value={levelsValue}>
        {children}
      </HuddleLevelsContext.Provider>
    </HuddleContext.Provider>
  );
}

export function useHuddle(): HuddleContextValue {
  const ctx = React.useContext(HuddleContext);
  if (!ctx) {
    throw new Error("useHuddle must be used within a HuddleProvider");
  }
  return ctx;
}

/**
 * High-frequency (20-30 Hz) mic/speaker levels. Consume only from components
 * that render audio meters; everything else should use {@link useHuddle} so it
 * is insulated from level churn.
 */
export function useHuddleLevels(): HuddleLevelsValue {
  const ctx = React.useContext(HuddleLevelsContext);
  if (!ctx) {
    throw new Error("useHuddleLevels must be used within a HuddleProvider");
  }
  return ctx;
}
