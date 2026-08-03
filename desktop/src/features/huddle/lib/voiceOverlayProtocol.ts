export type VoiceInputMode = "push_to_talk" | "voice_activity";

export const VOICE_OVERLAY_WINDOW_LABEL = "voice-overlay";
export const VOICE_OVERLAY_READY_EVENT = "buzz://voice-overlay/ready";
export const VOICE_OVERLAY_STATE_EVENT = "buzz://voice-overlay/state";
export const VOICE_OVERLAY_ACTION_EVENT = "buzz://voice-overlay/action";
export const VOICE_OVERLAY_ACTION_RESULT_EVENT =
  "buzz://voice-overlay/action-result";

export type VoiceOverlayActionInput =
  | { version: 1; type: "toggle_mute" }
  | {
      version: 1;
      type: "set_voice_input_mode";
      mode: VoiceInputMode;
    }
  | {
      version: 1;
      type: "toggle_transcription" | "toggle_tts" | "leave" | "show_main";
    };

export type VoiceOverlayAction = VoiceOverlayActionInput & {
  requestId: string;
};

export type VoiceOverlayActionResult =
  | { version: 1; requestId: string; ok: true }
  | { version: 1; requestId: string; ok: false; error: string };

export type VoiceOverlayActionHandlers = {
  onToggleMute: () => void | Promise<void>;
  onSetVoiceInputMode: (mode: VoiceInputMode) => void | Promise<void>;
  onToggleTranscription: () => void | Promise<void>;
  onToggleTts: () => void | Promise<void>;
  onLeave: () => void | Promise<void>;
  onShowMain: () => void | Promise<void>;
};

function voiceOverlayActionErrorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error ?? "");
  return (
    message.trim().slice(0, 240) ||
    "Voice action failed without an error message."
  );
}

export async function runVoiceOverlayAction(
  action: VoiceOverlayAction,
  handlers: VoiceOverlayActionHandlers,
): Promise<VoiceOverlayActionResult> {
  try {
    switch (action.type) {
      case "toggle_mute":
        await handlers.onToggleMute();
        break;
      case "set_voice_input_mode":
        await handlers.onSetVoiceInputMode(action.mode);
        break;
      case "toggle_transcription":
        await handlers.onToggleTranscription();
        break;
      case "toggle_tts":
        await handlers.onToggleTts();
        break;
      case "leave":
        await handlers.onLeave();
        break;
      case "show_main":
        await handlers.onShowMain();
        break;
    }
    return { version: 1, requestId: action.requestId, ok: true };
  } catch (error) {
    return {
      version: 1,
      requestId: action.requestId,
      ok: false,
      error: voiceOverlayActionErrorMessage(error),
    };
  }
}

type VoiceOverlayActionTrackerOptions = {
  setTimer: (callback: () => void, delayMs: number) => number;
  clearTimer: (timerId: number) => void;
  onSlow: (requestId: string) => void;
  onExpired: (requestId: string) => void;
  slowAfterMs?: number;
  expireAfterMs?: number;
};

export function createVoiceOverlayActionTracker({
  setTimer,
  clearTimer,
  onSlow,
  onExpired,
  slowAfterMs = 2_000,
  expireAfterMs = 30_000,
}: VoiceOverlayActionTrackerOptions) {
  const pending = new Map<
    string,
    { slowTimerId: number; expiryTimerId: number }
  >();

  const settle = (requestId: string): boolean => {
    const timers = pending.get(requestId);
    if (!timers) return false;
    pending.delete(requestId);
    clearTimer(timers.slowTimerId);
    clearTimer(timers.expiryTimerId);
    return true;
  };

  return {
    start(requestId: string) {
      settle(requestId);
      const slowTimerId = setTimer(() => {
        if (pending.has(requestId)) onSlow(requestId);
      }, slowAfterMs);
      const expiryTimerId = setTimer(() => {
        if (!settle(requestId)) return;
        onExpired(requestId);
      }, expireAfterMs);
      pending.set(requestId, { slowTimerId, expiryTimerId });
    },
    settle,
    dispose() {
      for (const requestId of [...pending.keys()]) settle(requestId);
    },
  };
}

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
  const requestId =
    typeof action.requestId === "string" ? action.requestId.trim() : "";
  if (
    action.version === 1 &&
    requestId.length > 0 &&
    requestId.length <= 128 &&
    [
      "toggle_mute",
      "toggle_transcription",
      "toggle_tts",
      "leave",
      "show_main",
    ].includes(String(action.type)) &&
    Object.keys(action).length === 3
  ) {
    return {
      version: 1,
      requestId,
      type: action.type as Exclude<
        VoiceOverlayAction["type"],
        "set_voice_input_mode"
      >,
    };
  }

  if (
    action.version === 1 &&
    requestId.length > 0 &&
    requestId.length <= 128 &&
    action.type === "set_voice_input_mode" &&
    Object.keys(action).length === 4 &&
    (action.mode === "push_to_talk" || action.mode === "voice_activity")
  ) {
    return {
      version: 1,
      requestId,
      type: action.type,
      mode: action.mode,
    };
  }

  return null;
}

export function parseVoiceOverlayActionResult(
  value: unknown,
): VoiceOverlayActionResult | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }

  const result = value as Record<string, unknown>;
  const requestId =
    typeof result.requestId === "string" ? result.requestId.trim() : "";
  if (
    result.version !== 1 ||
    requestId.length === 0 ||
    requestId.length > 128
  ) {
    return null;
  }

  if (result.ok === true && Object.keys(result).length === 3) {
    return { version: 1, requestId, ok: true };
  }

  const error = typeof result.error === "string" ? result.error.trim() : "";
  if (
    result.ok === false &&
    error.length > 0 &&
    Object.keys(result).length === 4
  ) {
    return { version: 1, requestId, ok: false, error };
  }

  return null;
}

export function voiceOverlayMediaSnapshot(
  state: VoiceOverlayMediaState,
): VoiceOverlayMediaState {
  const isIdle = state.phase === "idle";
  const isMuted = state.isMuted;
  return {
    version: 1,
    phase: state.phase,
    participantCount: isIdle ? 0 : state.participantCount,
    agentCount: isIdle ? 0 : state.agentCount,
    ttsEnabled: state.ttsEnabled,
    transcriptionEnabled: state.transcriptionEnabled,
    isLeaving: isIdle ? false : state.isLeaving,
    error: state.error,
    isMuted,
    micConnected: isIdle ? false : state.micConnected,
    micLevel: isIdle || isMuted ? 0 : Math.min(1, Math.max(0, state.micLevel)),
    pttActive: isIdle || isMuted ? false : state.pttActive,
    voiceInputMode: state.voiceInputMode,
  };
}
