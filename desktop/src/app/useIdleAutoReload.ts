import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { getOsIdleSeconds } from "@/shared/api/osIdle";
import { invokeTauri } from "@/shared/api/tauri";
import { isAppFocused } from "@/shared/lib/useDocumentVisible";
import {
  createSerializedIdleReloadCheck,
  IDLE_RELOAD_SESSION_AGE_MS,
  normalizeOsIdleMs,
  type IdleAutoReloadReaders,
} from "@/shared/lib/idleAutoReloadPolicy";
import { requestRendererReload } from "@/shared/lib/reloadRenderer";
import { isAnyVolatileWorkPending } from "@/shared/lib/volatileWorkRegistry";

/** Wall-clock ms when this renderer module first loaded — the session-age
 * anchor. `location.reload()` reloads the module, so a fired reload resets this
 * to ~now and re-arms the backstop automatically on the fresh page. */
const SESSION_LOADED_AT_MS = Date.now();

/** How often the backstop re-evaluates once the session is old enough to arm. */
const CHECK_INTERVAL_MS = 60_000;

/** True while a huddle is anything but fully idle. Unknown/errored → true so a
 * live call is never dropped by a reload. */
async function readHuddleActive(): Promise<boolean> {
  try {
    const state = await invokeTauri<{ phase?: string }>("get_huddle_state");
    return (state?.phase ?? "idle") !== "idle";
  } catch {
    return true;
  }
}

/**
 * Idle auto-reload backstop. Once the renderer session is ~12h old, checks
 * once a minute whether the user is provably away (OS idle + window unfocused)
 * with no volatile work outstanding, and if so performs the same clean-teardown
 * reload as Cmd+R — discarding the accumulated renderer heap and timers while
 * the native engine and its child agents keep running.
 *
 * The volatile-work blocker withholds the reload during a huddle, a foreground
 * or background media upload (including its completion settling window), queued
 * local attachments a draft cannot serialize, or any in-flight
 * send/mutation/native operation (`queryClient.isMutating()` — sends, media
 * pickers, install/import/save all run as mutations). Persisted draft text
 * survives reload via `localStorage`, so it is deliberately not a blocker.
 *
 * All fire/withhold logic lives in the pure, exhaustively tested policy module.
 * This hook only wires live signals, arms after the session-age threshold, and
 * serializes overlapping checks. Every signal is read imperatively at check
 * time (once a minute), so the hook holds no reactive subscription and never
 * re-renders.
 *
 * Mounted via {@link IdleAutoReloadController} — a null-rendering leaf inside
 * the main ready shell only. Huddle companion windows and every onboarding /
 * blocking / reset / keyring / relaunch screen are ineligible by construction.
 */
export function useIdleAutoReload(): void {
  const queryClient = useQueryClient();

  React.useEffect(() => {
    let disposed = false;
    let intervalId: number | undefined;
    let armTimerId: number | undefined;

    const readers: IdleAutoReloadReaders = {
      now: Date.now,
      sessionLoadedAtMs: SESSION_LOADED_AT_MS,
      isAppFocused,
      getOsIdleMs: () => normalizeOsIdleMs(getOsIdleSeconds),
      getHuddleActive: readHuddleActive,
      isVolatileWorkPending: () =>
        isAnyVolatileWorkPending() || queryClient.isMutating() > 0,
      reload: () => requestRendererReload(),
    };

    const check = createSerializedIdleReloadCheck(readers);

    const startInterval = () => {
      if (disposed) return;
      check();
      intervalId = window.setInterval(check, CHECK_INTERVAL_MS);
    };

    const msUntilArm =
      SESSION_LOADED_AT_MS + IDLE_RELOAD_SESSION_AGE_MS - Date.now();
    if (msUntilArm <= 0) {
      startInterval();
    } else {
      armTimerId = window.setTimeout(startInterval, msUntilArm);
    }

    return () => {
      disposed = true;
      if (intervalId !== undefined) window.clearInterval(intervalId);
      if (armTimerId !== undefined) window.clearTimeout(armTimerId);
    };
  }, [queryClient]);
}

/** Null-rendering mount point for {@link useIdleAutoReload}. Kept as its own
 * leaf so nothing in the app tree re-renders on its account. */
export function IdleAutoReloadController(): null {
  useIdleAutoReload();
  return null;
}
