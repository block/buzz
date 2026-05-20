import * as React from "react";

/**
 * Owns the "should we show the pre-join headphones warning?" decision.
 *
 * Wraps an arbitrary click handler — `gate(action)` returns a function that
 * either runs `action` directly (already confirmed this session) or shows
 * the confirmation dialog and runs `action` only after the user clicks
 * Continue. Cancel/Escape/click-outside all decline silently.
 *
 * The "confirmed this session" bit is stored in `sessionStorage`, so it
 * resets when the desktop window is closed but persists across channel
 * navigation in the same window. That matches Quinn's recipe in the
 * design thread: "session-dismissable, self-removing once AEC lands."
 *
 * `aecMissing` is a prop, not a feature-detect inside the hook, because
 * the AEC follow-up PR flips it from a parent constant; centralizing the
 * predicate makes the future deletion mechanical.
 *
 * Returns an object with `dialogOpen`, `gate(action)`, `onContinue`, and
 * `onCancel`. Render the `HeadphonesGate` component with the dialog props.
 */

const STORAGE_KEY = "huddle.headphones-gate-confirmed";

function readConfirmed(): boolean {
  try {
    return sessionStorage.getItem(STORAGE_KEY) === "1";
  } catch {
    // sessionStorage can throw in some embedded WebView contexts; treat
    // failure as "not confirmed" so the gate still warns conservatively.
    return false;
  }
}

function writeConfirmed() {
  try {
    sessionStorage.setItem(STORAGE_KEY, "1");
  } catch {
    /* best-effort; if storage is denied the user just sees the gate next time */
  }
}

export function useHeadphonesGate(aecMissing: boolean) {
  const [dialogOpen, setDialogOpen] = React.useState(false);
  const pendingActionRef = React.useRef<(() => void) | null>(null);

  const gate = React.useCallback(
    (action: () => void) => {
      // Once AEC works (predicate flips to false), this hook is a no-op:
      // run the action immediately. The component using this hook can stay
      // in place; deleting the hook + gate + notice happens in one diff
      // when the follow-up PR makes aecMissing always false.
      if (!aecMissing || readConfirmed()) {
        action();
        return;
      }
      pendingActionRef.current = action;
      setDialogOpen(true);
    },
    [aecMissing],
  );

  const onContinue = React.useCallback(() => {
    writeConfirmed();
    const action = pendingActionRef.current;
    pendingActionRef.current = null;
    setDialogOpen(false);
    action?.();
  }, []);

  const onCancel = React.useCallback(() => {
    pendingActionRef.current = null;
    setDialogOpen(false);
  }, []);

  return { dialogOpen, gate, onContinue, onCancel };
}
