import * as React from "react";

/**
 * Focus the composer editor on mount and whenever the active draft key
 * changes (channel switch, thread open).
 *
 * Matches the behaviour of Slack/Discord/Signal: the composer is ready to
 * accept typing without an explicit click. The `focus` callback is expected
 * to no-op until the underlying editor is mounted, and to change identity
 * once that happens — so listing it as a dep recovers from the
 * editor-not-ready-yet case on first render.
 *
 * The effect trigger deliberately excludes `disabled`: callers pass a
 * disabled flag that includes transient state like `isSending`, which would
 * otherwise re-fire autofocus after every send. When the main channel and
 * an open thread panel both have composers mounted, that race let the main
 * composer steal focus from the thread composer post-send. We only autofocus
 * on mount and on real navigation events (draft-key change).
 *
 * Guards:
 *  - Skip if the composer is currently disabled (archived channel, no
 *    channel, or in-flight send at the moment of mount).
 *  - Skip if focus already lives in another text-entry surface (open
 *    dialog input, search box, etc.) so we don't yank focus from the user.
 */
export function useComposerAutofocus(
  focus: () => void,
  draftKey: string | null | undefined,
  disabled: boolean,
) {
  // We read `disabled` at execution time but intentionally don't depend on
  // it — see the comment above.
  const disabledRef = React.useRef(disabled);
  disabledRef.current = disabled;

  // biome-ignore lint/correctness/useExhaustiveDependencies: draftKey is the trigger; disabled is read via ref
  React.useEffect(() => {
    if (disabledRef.current) return;
    if (typeof document === "undefined") return;
    const active = document.activeElement as HTMLElement | null;
    if (active && active !== document.body) {
      const tag = active.tagName;
      if (
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "SELECT" ||
        active.isContentEditable
      ) {
        return;
      }
    }
    focus();
  }, [draftKey, focus]);
}
