export type VoiceInputMode = "push_to_talk" | "voice_activity";

export const VOICE_OVERLAY_WINDOW_LABEL = "voice-overlay";
export const VOICE_OVERLAY_READY_EVENT = "buzz://voice-overlay/ready";
export const VOICE_OVERLAY_STATE_EVENT = "buzz://voice-overlay/state";
export const VOICE_OVERLAY_ACTION_EVENT = "buzz://voice-overlay/action";

export type VoiceOverlayAction =
  | { version: 1; type: "toggle_mute" }
  | { version: 1; type: "set_voice_input_mode"; mode: VoiceInputMode }
  | {
      version: 1;
      type: "toggle_transcription" | "toggle_tts" | "leave" | "show_main";
    };

export type VoiceOverlayPhase =
  | "idle"
  | "creating"
  | "connecting"
  | "connected"
  | "active"
  | "leaving";

export type VoiceOverlayMediaState = {
  version: 1;
  phase: VoiceOverlayPhase;
  participantCount: number;
  agentCount: number;
  ttsEnabled: boolean;
  transcriptionEnabled: boolean;
  isLeaving: boolean;
  error: string | null;
  isMuted: boolean;
  micConnected: boolean;
  micLevel: number;
  pttActive: boolean;
  voiceInputMode: VoiceInputMode;
};

export function parseVoiceOverlayAction(
  value: unknown,
): VoiceOverlayAction | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }

  const action = value as Record<string, unknown>;
  if (
    action.version === 1 &&
    [
      "toggle_mute",
      "toggle_transcription",
      "toggle_tts",
      "leave",
      "show_main",
    ].includes(String(action.type)) &&
    Object.keys(action).length === 2
  ) {
    return {
      version: 1,
      type: action.type as Exclude<
        VoiceOverlayAction["type"],
        "set_voice_input_mode"
      >,
    };
  }

  if (
    action.version === 1 &&
    action.type === "set_voice_input_mode" &&
    Object.keys(action).length === 3 &&
    (action.mode === "push_to_talk" || action.mode === "voice_activity")
  ) {
    return { version: 1, type: action.type, mode: action.mode };
  }

  return null;
}

export function voiceOverlayMediaSnapshot(
  state: VoiceOverlayMediaState,
): VoiceOverlayMediaState {
  const isMuted = state.isMuted;
  return {
    version: 1,
    phase: state.phase,
    participantCount: state.participantCount,
    agentCount: state.agentCount,
    ttsEnabled: state.ttsEnabled,
    transcriptionEnabled: state.transcriptionEnabled,
    isLeaving: state.isLeaving,
    error: state.error,
    isMuted,
    micConnected: state.micConnected,
    micLevel: isMuted ? 0 : Math.min(1, Math.max(0, state.micLevel)),
    pttActive: isMuted ? false : state.pttActive,
    voiceInputMode: state.voiceInputMode,
  };
}
