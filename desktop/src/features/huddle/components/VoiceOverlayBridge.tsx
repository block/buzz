import { emitTo, listen } from "@tauri-apps/api/event";
import * as React from "react";

import {
  parseVoiceOverlayAction,
  type VoiceInputMode,
  type VoiceOverlayMediaState,
  VOICE_OVERLAY_ACTION_EVENT,
  VOICE_OVERLAY_READY_EVENT,
  VOICE_OVERLAY_STATE_EVENT,
  VOICE_OVERLAY_WINDOW_LABEL,
} from "../lib/voiceOverlayProtocol";

type VoiceOverlayBridgeProps = {
  snapshot: VoiceOverlayMediaState;
  onToggleMute: () => void | Promise<void>;
  onSetVoiceInputMode: (mode: VoiceInputMode) => void | Promise<void>;
  onToggleTranscription: () => void | Promise<void>;
  onToggleTts: () => void | Promise<void>;
  onLeave: () => void | Promise<void>;
  onShowMain: () => void | Promise<void>;
};

export function VoiceOverlayBridge({
  snapshot,
  onToggleMute,
  onSetVoiceInputMode,
  onToggleTranscription,
  onToggleTts,
  onLeave,
  onShowMain,
}: VoiceOverlayBridgeProps) {
  const snapshotRef = React.useRef(snapshot);
  snapshotRef.current = snapshot;

  const handlersRef = React.useRef({
    onToggleMute,
    onSetVoiceInputMode,
    onToggleTranscription,
    onToggleTts,
    onLeave,
    onShowMain,
  });
  handlersRef.current = {
    onToggleMute,
    onSetVoiceInputMode,
    onToggleTranscription,
    onToggleTts,
    onLeave,
    onShowMain,
  };

  const publishSnapshot = React.useCallback((next: VoiceOverlayMediaState) => {
    void emitTo(
      VOICE_OVERLAY_WINDOW_LABEL,
      VOICE_OVERLAY_STATE_EVENT,
      next,
    ).catch(() => {
      // The floating controller is optional and may not be open.
    });
  }, []);

  React.useEffect(() => {
    publishSnapshot(snapshot);
  }, [publishSnapshot, snapshot]);

  React.useEffect(() => {
    let disposed = false;
    const cleanups: Array<() => void> = [];

    void listen(VOICE_OVERLAY_READY_EVENT, () => {
      if (!disposed) publishSnapshot(snapshotRef.current);
    }).then((unlisten) => {
      if (disposed) void unlisten();
      else cleanups.push(() => void unlisten());
    });

    void listen(VOICE_OVERLAY_ACTION_EVENT, (event) => {
      if (disposed) return;
      const action = parseVoiceOverlayAction(event.payload);
      if (!action) return;

      const handlers = handlersRef.current;
      switch (action.type) {
        case "toggle_mute":
          void handlers.onToggleMute();
          break;
        case "set_voice_input_mode":
          void handlers.onSetVoiceInputMode(action.mode);
          break;
        case "toggle_transcription":
          void handlers.onToggleTranscription();
          break;
        case "toggle_tts":
          void handlers.onToggleTts();
          break;
        case "leave":
          void handlers.onLeave();
          break;
        case "show_main":
          void handlers.onShowMain();
          break;
      }
    }).then((unlisten) => {
      if (disposed) void unlisten();
      else cleanups.push(() => void unlisten());
    });

    return () => {
      disposed = true;
      for (const cleanup of cleanups) cleanup();
    };
  }, [publishSnapshot]);

  return null;
}
