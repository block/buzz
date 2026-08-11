import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import * as React from "react";

import { setupAudioWorklet, type AudioWorkletHandle } from "./lib/audioWorklet";
import { type AudioInputDevice, useAudioDevices } from "./lib/useAudioDevices";
import { usePipelineHotstart } from "./lib/usePipelineHotstart";
import { formatHuddleActionError } from "./lib/huddleError";
import {
  type VoiceInputMode,
  useHuddlePttState,
} from "./lib/useHuddlePttState";
import { useHuddleSpeakerActivity } from "./lib/useHuddleSpeakerActivity";
import { useTtsSubscription } from "./lib/useTtsSubscription";
import type { HuddleContextValue } from "./HuddleContext.types";

/**
 * Huddle lifecycle (React context):
 *   startHuddle/joinHuddle → invoke(start/join_huddle) → getUserMedia + setupAudioWorklet
 *     → confirm_huddle_active
 *   TTS subscription: subscribeToChannelLive → filter agent pubkeys → speak_agent_message
 *   leaveHuddle: stop worklet → stop mic track → invoke(leave_huddle)
 *   Active speakers: Tauri "huddle-active-speakers" event (Rust backend emits)
 */

type HuddleJoinInfo = {
  ephemeral_channel_id: string;
};

type HuddleAudioMirrorState = {
  isMuted: boolean;
  micConnected: boolean;
  audioDevices: AudioInputDevice[];
  selectedDeviceId: string;
  micGain: number;
  voiceInputMode: VoiceInputMode;
};

type HuddleAudioCommand =
  | { type: "request-state" }
  | { type: "set-muted"; isMuted: boolean }
  | { type: "set-input-device"; deviceId: string }
  | { type: "set-mic-gain"; gain: number }
  | { type: "set-voice-input-mode"; mode: VoiceInputMode };

const HUDDLE_AUDIO_COMMAND_EVENT = "huddle-audio-command";
const HUDDLE_AUDIO_STATE_EVENT = "huddle-audio-state";
const HUDDLE_AUDIO_LEVEL_EVENT = "huddle-audio-level";

const MIC_ANALYSER_UPDATE_INTERVAL_MS = 33;
const MIC_INITIAL_NOISE_FLOOR = 0.01;
const MIC_VOICE_GATE_ON_RMS = 0.018;
const MIC_VOICE_GATE_OFF_RMS = 0.012;
const MIC_VOICE_GATE_MARGIN_RMS = 0.012;
const MIC_LEVEL_ACTIVE_RANGE_RMS = 0.11;
const MIC_MIN_ACTIVE_LEVEL = 0.18;
const MIC_LEVEL_ATTACK = 0.58;
const MIC_ACTIVE_NOISE_FLOOR_RISE = 0.006;

function isRedundantHuddlePhaseError(message: string): boolean {
  return /^cannot (?:start|join) huddle: already in phase /i.test(message);
}

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}

function interruptAgentSpeech(agentPubkey: string) {
  return invoke<void>("interrupt_huddle_speech", { agentPubkey });
}

const HuddleContext = React.createContext<HuddleContextValue | null>(null);

export function HuddleProvider({
  children,
  ownsAudioSession = true,
  onHuddleStartPendingChange,
  onHuddleStarted,
  onShowHuddleInMainApp,
  onViewHuddleChannel,
}: {
  children: React.ReactNode;
  /** A companion huddle window mirrors the session but must not end it on close. */
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
  const [micLevel, setMicLevel] = React.useState(0);
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
  const setVoiceInputMode = React.useCallback(
    async (mode: VoiceInputMode) => {
      await invoke("set_voice_input_mode", { mode });
      setVoiceInputModeState(mode);
      if (ownsAudioSession) {
        workletRef.current?.setMode(mode);
      } else {
        void emit(HUDDLE_AUDIO_COMMAND_EVENT, {
          type: "set-voice-input-mode",
          mode,
        } satisfies HuddleAudioCommand);
      }
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
        setVoiceInputModeState(event.payload.mode);
        workletRef.current?.setMode(event.payload.mode);
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

  // Keep the browser-owned session keyed to Rust across provider remounts. A
  // restored main window has not called connectAndSetupMedia, so without this
  // hydration its active Huddle channel is mistaken for a normal channel.
  // The companion can also end the native Huddle while the main window still
  // owns capture; release that shared capture as soon as Rust announces Idle.
  React.useEffect(() => {
    if (!ownsAudioSession) return;

    type HuddleBackendState = {
      phase?: string;
      ephemeral_channel_id?: string | null;
    };

    const applyBackendState = (state: HuddleBackendState) => {
      if (state.phase === "idle") {
        void disconnectMedia();
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
        const worklet = await setupAudioWorklet(
          audioTrack,
          getVoiceInputMode(),
          !isMutedRef.current,
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

      isMutedRef.current = false;
      setIsMuted(false);
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
        try {
          await connectAndSetupMedia(joinInfo, myToken);
        } catch (e) {
          if (e instanceof Error && e.message === "superseded") {
            cleanupSupersededStart(workletRef.current);
            return;
          }
          throw e;
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
      onHuddleStartPendingChange,
      onHuddleStarted,
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
      isMutedRef.current = false;
      setIsMuted(false);
      setHuddleError(null);
      setIsStarting(true);

      try {
        const joinInfo = await invoke<HuddleJoinInfo>("join_huddle", {
          parentChannelId,
          ephemeralChannelId,
          huddleThreadEventId,
        });
        rustActiveRef.current = true;

        try {
          await connectAndSetupMedia(joinInfo, myToken);
        } catch (e) {
          if (e instanceof Error && e.message === "superseded") {
            cleanupSupersededStart(workletRef.current);
            return;
          }
          throw e;
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
          return;
        }

        const w = workletRef.current;
        workletRef.current = null;
        await cleanupFailedStart(w, false);
        setHuddleError(formatHuddleActionError(e, "join"));
        console.error("Failed to join huddle:", e);
        throw e;
      } finally {
        setIsStarting(false);
        busyRef.current = false;
      }
    },
    [
      cleanupFailedStart,
      cleanupSupersededStart,
      connectAndSetupMedia,
      onHuddleStarted,
    ],
  );

  // The main window owns the browser audio session and therefore the one TTS
  // subscription. Companion windows receive native playback activity events,
  // but must not enqueue the same reply a second time.
  useTtsSubscription(
    ownsAudioSession ? ephemeralChannelId : null,
    selfPubkeyRef,
  );

  usePipelineHotstart(ephemeralChannelId);

  // Mic level analyser — drives the voice activity indicator
  React.useEffect(() => {
    if (!localAudioTrack || !micConnected) {
      setMicLevel(0);
      return;
    }

    const ctx = new AudioContext();
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 512;
    const source = ctx.createMediaStreamSource(
      new MediaStream([localAudioTrack]),
    );
    source.connect(analyser);
    const buf = new Float32Array(analyser.fftSize);

    let raf = 0;
    let lastUpdate = 0;
    let voiceActive = false;
    let noiseFloor = MIC_INITIAL_NOISE_FLOOR;
    let smoothedLevel = 0;
    function tick(now: number) {
      raf = requestAnimationFrame(tick);
      if (now - lastUpdate < MIC_ANALYSER_UPDATE_INTERVAL_MS) return;
      lastUpdate = now;
      analyser.getFloatTimeDomainData(buf);

      let sumSquares = 0;
      for (let i = 0; i < buf.length; i += 1) {
        sumSquares += buf[i] * buf[i];
      }

      const rms = Math.sqrt(sumSquares / buf.length);
      const activeThreshold = Math.max(
        MIC_VOICE_GATE_ON_RMS,
        noiseFloor + MIC_VOICE_GATE_MARGIN_RMS,
      );
      const idleThreshold = Math.max(
        MIC_VOICE_GATE_OFF_RMS,
        noiseFloor + MIC_VOICE_GATE_MARGIN_RMS * 0.55,
      );
      voiceActive = voiceActive ? rms > idleThreshold : rms > activeThreshold;

      const floorRate =
        rms < noiseFloor
          ? 0.18
          : voiceActive
            ? MIC_ACTIVE_NOISE_FLOOR_RISE
            : 0.025;
      noiseFloor += (rms - noiseFloor) * floorRate;

      if (!voiceActive) {
        smoothedLevel = 0;
        setMicLevel(0);
        return;
      }

      const normalized = clamp01(
        (rms - noiseFloor) / MIC_LEVEL_ACTIVE_RANGE_RMS,
      );
      const targetLevel = Math.max(normalized, MIC_MIN_ACTIVE_LEVEL);
      smoothedLevel += (targetLevel - smoothedLevel) * MIC_LEVEL_ATTACK;
      setMicLevel(smoothedLevel);
    }
    raf = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(raf);
      source.disconnect();
      void ctx.close();
    };
  }, [localAudioTrack, micConnected]);

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

  // Cleanup on unmount only — stable ref prevents re-firing mid-startup.
  const leaveHuddleRef = React.useRef(leaveHuddle);
  leaveHuddleRef.current = leaveHuddle;
  React.useEffect(() => {
    if (!ownsAudioSession) return;
    return () => {
      void leaveHuddleRef.current();
    };
  }, [ownsAudioSession]);

  // Unexpected audio-owner/pod disconnects are recoverable: keep the huddle,
  // mic, and voice pipelines live while Rust reconnects only the audio WS.
  // `tokenRef` makes an intentional leave/start supersede this loop, and the
  // in-flight guard collapses duplicate disconnect events from failed dials.
  const audioReconnectInFlightRef = React.useRef(false);
  React.useEffect(() => {
    if (!ownsAudioSession) return;

    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen("huddle-audio-disconnected", () => {
      if (cancelled || audioReconnectInFlightRef.current) return;
      audioReconnectInFlightRef.current = true;
      const reconnectToken = tokenRef.current;

      void (async () => {
        // Keep a long enough tail for Kubernetes Service endpoint removal after
        // a draining pod flips readiness. Early retries make remote-owner
        // handoff fast; the two 2s attempts prevent a client connected to the
        // draining pod itself from exhausting before kube-proxy converges.
        const delaysMs = [0, 100, 250, 500, 1_000, 2_000, 2_000];
        for (const delayMs of delaysMs) {
          if (cancelled || tokenRef.current !== reconnectToken) return;
          if (delayMs > 0) {
            await new Promise((resolve) => window.setTimeout(resolve, delayMs));
          }
          if (cancelled || tokenRef.current !== reconnectToken) return;
          try {
            await invoke("reconnect_huddle_audio");
            // Success installs a live replacement pipeline. If it later fails,
            // its Tauri event arrives after this loop releases the in-flight
            // guard and starts a fresh bounded recovery cycle. Repeating those
            // cycles is intentional while the relay remains connectable.
            return;
          } catch {
            // A draining pod may still receive the first retry before Service
            // endpoints converge. Keep the bounded backoff client-local.
          }
        }

        if (!cancelled && tokenRef.current === reconnectToken) {
          await leaveHuddleRef.current();
        }
      })().finally(() => {
        audioReconnectInFlightRef.current = false;
      });
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [ownsAudioSession]);

  return (
    <HuddleContext.Provider
      value={{
        localAudioTrack,
        isStarting,
        huddleError,
        clearHuddleError,
        micConnected: ownsAudioSession
          ? micConnected
          : (mirroredAudioState?.micConnected ?? false),
        isMuted: effectiveIsMuted,
        toggleMute,
        interruptAgentSpeech,
        micLevel: ownsAudioSession ? micLevel : mirroredMicLevel,
        pttActive,
        voiceInputMode: effectiveVoiceInputMode,
        setVoiceInputMode,
        activeSpeakers,
        speakerLevels,
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
      }}
    >
      {children}
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
