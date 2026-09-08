import * as React from "react";
import { listen } from "@tauri-apps/api/event";

/** Mirror of the Rust `InstallOutputEvent` payload (install_report.rs). */
export type InstallOutputEvent = {
  runtime_id: string;
  /** Monotonic across the whole install, not per step or per attempt. */
  seq: number;
  /** Null is the start signal: clear the displayed line now. */
  line: string | null;
};

/** The line being shown, and the sequence number that produced it. */
export type InstallOutputState = {
  seq: number;
  line: string | null;
};

/**
 * Fold one event into the displayed line.
 *
 * Events from another runtime are ignored — every install card listens to the
 * same channel. So is an out-of-order event: emission is monotonic in `seq`, so
 * a lower one has already been superseded. That matters at a retry boundary,
 * where a line emitted just as the next attempt starts would otherwise sit
 * under the spinner showing the failure the user already had.
 *
 * The ordering key is the install-wide `seq` rather than the attempt number,
 * which restarts at 1 for every step: keyed on attempt, a step that succeeded on
 * attempt 2 would make the next step's attempt-1 output look stale and freeze
 * the display for the rest of the install.
 */
export function nextInstallOutputLine(
  current: InstallOutputState | null,
  event: InstallOutputEvent,
  runtimeId: string,
): InstallOutputState | null {
  if (event.runtime_id !== runtimeId) return current;
  if (current && event.seq <= current.seq) return current;
  return { seq: event.seq, line: event.line };
}

/**
 * The install command's most recent output line for `runtimeId`, or null when
 * nothing is being shown — the install is not running, nothing has printed yet,
 * or the backend cleared the line because a new attempt is starting.
 *
 * An install runs for up to 15 minutes with no other feedback than a spinner;
 * this turns that wait into observable progress. The backend throttles
 * emission, so this re-renders a few times a second at most.
 *
 * Pass `isInstalling` so the line clears when the install settles — a finished
 * install must not leave its last line under a fresh Install button.
 */
export function useInstallOutputLine(
  runtimeId: string,
  isInstalling: boolean,
): string | null {
  const [state, setState] = React.useState<InstallOutputState | null>(null);

  // Subscribed for this runtime's whole lifetime, not just while installing.
  // The install command is invoked from the click handler, so the backend can
  // emit the attempt-start clear and the first line before React commits
  // `isInstalling` — and there is no replay, so a subscription that waited for
  // that commit would lose those events permanently. A fast command's entire
  // output is exactly what fits in that window.
  React.useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    (async () => {
      try {
        const stop = await listen<InstallOutputEvent>(
          "acp-install-output",
          (event) => {
            if (cancelled) return;
            setState((current) =>
              nextInstallOutputLine(current, event.payload, runtimeId),
            );
          },
        );
        if (cancelled) {
          stop();
        } else {
          unlisten = stop;
        }
      } catch {
        // Event system unavailable (web/e2e) — the spinner shows alone.
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [runtimeId]);

  // `seq` is monotonic within one install and restarts at 0 for the next, so
  // state must not outlive the run that produced it: a retained higher `seq`
  // would make every event of the following install look superseded. Settling
  // is the run boundary, so it is where the ordering key resets.
  React.useEffect(() => {
    if (!isInstalling) setState(null);
  }, [isInstalling]);

  // Events that arrive after the install settles — a drain flushing its last
  // line — must not reappear under a fresh Install button, so the line is
  // reported only while the install is running.
  return (isInstalling ? state?.line : null) ?? null;
}
