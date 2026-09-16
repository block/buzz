/**
 * Global back/forward navigation chords.
 *
 * macOS: ⌘[ / ⌘] — matching Safari, Chrome, Finder, and Slack.
 * Windows/Linux: Alt+← / Alt+→ — matching browsers and Slack.
 *
 * Kept pure (no DOM access) so chord matching can be unit tested; the
 * window listener wiring lives in `useBackForwardControls`.
 */

export type BackForwardDirection = "back" | "forward";

export type BackForwardChordEvent = Pick<
  KeyboardEvent,
  "altKey" | "code" | "ctrlKey" | "key" | "metaKey" | "shiftKey"
>;

export function matchBackForwardChord(
  event: BackForwardChordEvent,
  isMac: boolean,
): BackForwardDirection | null {
  if (isMac) {
    if (!event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) {
      return null;
    }

    if (event.key === "[" || event.code === "BracketLeft") {
      return "back";
    }

    if (event.key === "]" || event.code === "BracketRight") {
      return "forward";
    }

    return null;
  }

  if (!event.altKey || event.metaKey || event.ctrlKey || event.shiftKey) {
    return null;
  }

  if (event.key === "ArrowLeft") {
    return "back";
  }

  if (event.key === "ArrowRight") {
    return "forward";
  }

  return null;
}
