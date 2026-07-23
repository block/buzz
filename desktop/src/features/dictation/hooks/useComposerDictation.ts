import type * as React from "react";
import { useEffect, useId, useRef } from "react";
import {
  clearActiveDictationComposer,
  isActiveDictationComposer,
  setActiveDictationComposer,
} from "../lib/activeComposer";
import { useDictation } from "./useDictation";

interface UseComposerDictationOptions {
  /** Whether the voice-dictation preview feature is enabled. */
  enabled?: boolean;
  /** Ref to a function that syncs contentRef from the Tiptap editor and returns it. */
  syncContentRef: React.MutableRefObject<() => string>;
  /** Whether the composer is currently disabled (read-only, etc.). */
  disabled?: boolean;
  disabledRef: React.MutableRefObject<boolean>;
  isSendingRef: React.MutableRefObject<boolean>;
  isUploadingRef: React.MutableRefObject<boolean>;
  /** Updates contentRef + isContentEmpty state. */
  setComposerContent: (text: string) => void;
  /** Ref to a function that updates the Tiptap editor document. */
  setEditorContentRef: React.MutableRefObject<(text: string) => void>;
  submitMessageRef: React.MutableRefObject<() => void>;
  /** When this key changes (channel/thread switch), active dictation is stopped. */
  draftKey?: string | null;
  /** Ref to the composer's container element for focus tracking. */
  composerRef?: React.RefObject<HTMLElement | null>;
}

/**
 * Thin wrapper around `useDictation` pre-wired for the MessageComposer's
 * state management (syncContentRef, setComposerContent, editor, submitMessageRef).
 *
 * Uses the local Parakeet STT engine — fully offline, no relay or API key needed.
 */
export function useComposerDictation({
  enabled = true,
  syncContentRef,
  disabled = false,
  disabledRef,
  isSendingRef,
  isUploadingRef,
  setComposerContent,
  setEditorContentRef,
  submitMessageRef,
  draftKey,
  composerRef,
}: UseComposerDictationOptions) {
  const instanceId = useId();
  const isSendBlockedRef = useRef(false);
  isSendBlockedRef.current =
    !enabled ||
    disabledRef.current ||
    isSendingRef.current ||
    isUploadingRef.current;

  const dictation = useDictation({
    disabled: !enabled,
    getText: () => syncContentRef.current(),
    setText: (text) => {
      setComposerContent(text);
      setEditorContentRef.current(text);
    },
    onSend: (text) => {
      setComposerContent(text);
      setEditorContentRef.current(text);
      // Submit synchronously — the content ref is already set above, so
      // syncComposerContentFromEditor() will serialize the editor which now
      // holds the dictated text.
      submitMessageRef.current();
    },
    isSendBlockedRef,
  });

  // Track which composer is active (most recently focused) so that the global
  // ⌘D shortcut only dispatches to one instance when multiple are mounted.
  useEffect(() => {
    const el = composerRef?.current;
    if (!el) {
      // If no ref provided, this composer is always considered active (single-composer case).
      setActiveDictationComposer(instanceId);
      return () => clearActiveDictationComposer(instanceId);
    }

    function handleFocusIn() {
      setActiveDictationComposer(instanceId);
    }

    el.addEventListener("focusin", handleFocusIn);
    return () => {
      el.removeEventListener("focusin", handleFocusIn);
      clearActiveDictationComposer(instanceId);
    };
  }, [instanceId, composerRef]);

  // Cancel dictation when the channel/thread changes so that transcript events
  // from a stale local STT session don't leak into the wrong draft.
  // Only cancel if this instance owns a live/pending session — avoids killing
  // another composer's session since the native engine is a singleton.
  //
  // Includes `isTranscribing`, not just `isRecording`/`isStarting`:
  // `stopRecording()` clears `isRecording` immediately but deliberately keeps
  // the transcript listener alive (isTranscribing=true) until the native
  // `stopped` event, so the final local STT flush can still be appended. If the
  // draftKey changes during that grace window, we must still cancel — otherwise
  // the late transcript is appended through this same composer instance into the
  // newly restored draft. `cancelRecording()` unlistens the transcript handler
  // and performs a session-scoped native stop, so this is safe.
  const isOwningSessionRef = useRef(false);
  isOwningSessionRef.current =
    dictation.isRecording || dictation.isStarting || dictation.isTranscribing;
  // biome-ignore lint/correctness/useExhaustiveDependencies: draftKey is the sole trigger
  useEffect(() => {
    if (isOwningSessionRef.current) {
      dictation.cancelRecording();
    }
  }, [draftKey]);

  // Auto-cancel dictation when the composer becomes disabled mid-session
  // (e.g. channel becomes read-only, parent send state disables thread composer).
  // Without this, the STT session keeps running with no way to stop it.
  //
  // Covers the full ownership window — `isStarting` and `isTranscribing`, not
  // just `isRecording`. If the composer is disabled while `startRecording()` is
  // still resolving (mic permission / AudioWorklet setup), the pending start
  // would otherwise finish and begin microphone capture in a disabled composer.
  // `cancelRecording()` aborts the in-flight start as well as a live recording,
  // and re-running when any of these flags flip catches a start that completes
  // after the disable.
  useEffect(() => {
    const owningSession =
      dictation.isRecording || dictation.isStarting || dictation.isTranscribing;
    if ((!enabled || disabled) && owningSession) {
      dictation.cancelRecording();
    }
  }, [
    enabled,
    disabled,
    dictation.isRecording,
    dictation.isStarting,
    dictation.isTranscribing,
    dictation.cancelRecording,
  ]);

  // ⌘D push-to-talk — hold to record, release to stop.
  // Dispatched from AppShell's keydown/keyup handlers.
  // Only the active (most recently focused) composer responds, and only while
  // focus remains inside it and it is not disabled/send-blocked.
  // biome-ignore lint/correctness/useExhaustiveDependencies: disabledRef/isSendBlockedRef are stable refs read at call time
  useEffect(() => {
    function handleKeyDown() {
      if (!enabled) return;
      // Only respond if this is the active composer instance.
      if (!isActiveDictationComposer(instanceId)) return;
      // Only respond if focus is still inside this composer. The active-composer
      // registration persists after focusout (until unmount), so without this
      // check, focusing a composer and then moving to another mounted input or
      // dialog (quick search, create-channel, etc.) would still let ⌘D start
      // microphone capture and append transcript into the background draft.
      const el = composerRef?.current;
      if (el && !el.contains(document.activeElement)) return;
      // Don't start dictation in disabled/blocked composers.
      if (disabledRef.current || isSendBlockedRef.current) return;
      if (!dictation.isRecording && !dictation.isStarting) {
        dictation.startRecording();
      }
    }
    function handleKeyUp() {
      if (dictation.isRecording || dictation.isStarting) {
        dictation.stopRecording();
      }
    }
    window.addEventListener("buzz:dictation-key-down", handleKeyDown);
    window.addEventListener("buzz:dictation-key-up", handleKeyUp);
    return () => {
      window.removeEventListener("buzz:dictation-key-down", handleKeyDown);
      window.removeEventListener("buzz:dictation-key-up", handleKeyUp);
    };
  }, [
    instanceId,
    composerRef,
    enabled,
    dictation.isRecording,
    dictation.isStarting,
    dictation.startRecording,
    dictation.stopRecording,
  ]);

  return dictation;
}
