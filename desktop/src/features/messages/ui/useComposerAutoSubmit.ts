import * as React from "react";

/**
 * Auto-submit on draft send: when `autoSubmitDraftKey` is set (the user
 * clicked "Send message" in the Drafts panel and confirmed), fire the
 * composer's submit once after mount so the draft is sent through the real
 * send path (mention resolution, media, etc.).
 *
 * Guard: only fires when the effective draft key matches the trigger so a
 * stale URL param on a different channel never fires a spurious send.
 *
 * Fires at most once per mount (empty dep array after the key check) — the
 * `onAutoSubmitComplete` callback clears the trigger before the submit runs,
 * preventing re-fire on re-render or back-navigation.
 */
export function useComposerAutoSubmit({
  autoSubmitDraftKey,
  effectiveDraftKey,
  onAutoSubmitComplete,
  submitMessageRef,
}: {
  autoSubmitDraftKey: string | null;
  effectiveDraftKey: string | null;
  onAutoSubmitComplete?: () => void;
  submitMessageRef: React.RefObject<() => void>;
}) {
  const onAutoSubmitCompleteRef = React.useRef(onAutoSubmitComplete);
  onAutoSubmitCompleteRef.current = onAutoSubmitComplete;

  // biome-ignore lint/correctness/useExhaustiveDependencies: intentionally fires once on mount only
  React.useEffect(() => {
    if (
      autoSubmitDraftKey === null ||
      autoSubmitDraftKey !== effectiveDraftKey
    ) {
      return;
    }
    // Clear the trigger BEFORE firing so any navigation from the send cannot
    // loop back with the param still present.
    onAutoSubmitCompleteRef.current?.();
    // Defer by one macrotask so the draft-persist lifecycle effect (which runs
    // synchronously after mount) has a chance to load the draft content into
    // the Tiptap editor before we try to submit.
    const timer = window.setTimeout(() => {
      submitMessageRef.current();
    }, 0);
    return () => {
      window.clearTimeout(timer);
    };
  }, []); // mount-only
}
