import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as React from "react";
import { toast } from "sonner";

import { useHuddle } from "@/features/huddle";
import {
  applyPlaybackEvent,
  IDLE_STATE,
  isMessagePlaying,
  type MessageTtsBackendEvent,
  type MessageTtsUiState,
} from "@/features/message-tts/lib/playbackReducer.mjs";

/**
 * App-scoped read-aloud state for chat messages.
 *
 * One playback app-wide: pressing play on message B while A speaks
 * supersedes A (the backend cancels A's audio and emits "superseded"; the
 * reducer keeps state keyed to B). All state transitions come FROM the
 * backend via `message-tts-state-changed` — the provider never guesses;
 * a play click shows feedback only when the backend confirms "preparing".
 */
type MessageTtsContextValue = {
  /** Whether `messageId` currently owns playback (preparing or playing). */
  isPlaying: (messageId: string) => boolean;
  /** True while the current playback's text was cut at the length limit. */
  isTruncated: (messageId: string) => boolean;
  /** Start reading `body` aloud, superseding any current playback. */
  play: (messageId: string, body: string) => void;
  /** Stop the current playback (no-op when nothing is playing). */
  stop: () => void;
  /** Non-null while read-aloud is unavailable (e.g. during a huddle). */
  disabledReason: string | null;
};

const MessageTtsContext = React.createContext<MessageTtsContextValue | null>(
  null,
);

export function useMessageTts(): MessageTtsContextValue | null {
  // Nullable on purpose: surfaces (like web builds or tests) without the
  // provider simply hide the read-aloud affordance.
  return React.useContext(MessageTtsContext);
}

export function MessageTtsProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const [state, setState] = React.useState<MessageTtsUiState>(IDLE_STATE);
  // Provider is mounted inside HuddleProvider in AppShell. A live huddle owns
  // the audio device, so read-aloud is disabled while connected (the backend
  // enforces this too — the UI gate just explains it before the click).
  const { activeEphemeralChannelId } = useHuddle();
  const disabledReason = activeEphemeralChannelId
    ? "Read aloud is unavailable during a huddle"
    : null;

  React.useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;

    listen<MessageTtsBackendEvent>("message-tts-state-changed", (event) => {
      if (cancelled) {
        return;
      }
      setState((prev) => {
        const { state: next, detachedError } = applyPlaybackEvent(
          prev,
          event.payload,
        );
        if (detachedError) {
          toast.error(`Read aloud failed: ${detachedError}`);
        }
        // Long-message limit disclosure: the backend caps very long messages
        // and reports the cut via `truncated` (the audio also ends with a
        // spoken "message truncated"). Announce once, when playback starts.
        if (event.payload.status === "preparing" && event.payload.truncated) {
          toast.info("This message is long — only the beginning will be read.");
        }
        return next;
      });
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const play = React.useCallback((messageId: string, body: string) => {
    // Errors also arrive as "failed" events (handled above with a toast);
    // the rejected promise carries the same text, so swallow it here.
    invoke("speak_chat_message", { messageId, text: body }).catch(() => {});
  }, []);

  const stop = React.useCallback(() => {
    invoke("stop_chat_message_speech").catch(() => {});
  }, []);

  const contextValue = React.useMemo<MessageTtsContextValue>(
    () => ({
      isPlaying: (messageId: string) => isMessagePlaying(state, messageId),
      isTruncated: (messageId: string) =>
        state.messageId === messageId && state.truncated,
      play,
      stop,
      disabledReason,
    }),
    [state, play, stop, disabledReason],
  );

  return (
    <MessageTtsContext.Provider value={contextValue}>
      {children}
    </MessageTtsContext.Provider>
  );
}
