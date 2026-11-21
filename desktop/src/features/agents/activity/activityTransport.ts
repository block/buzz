// Ported/adapted from Berd's agent-activity-panel (branch
// zmarley/agent-activity-panel). DVR-style transport state for the activity
// panel: LIVE tracks the scene's growing edge, REPLAY scrubs display time at
// selectable speeds, and playback rejoins LIVE when it catches up.

export type ActivitySpeed = 1 | 2 | 4 | 8;
export type ActivityTransportMode = "live" | "replay";

export interface ActivityTransportState {
  mode: ActivityTransportMode;
  playing: boolean;
  speed: ActivitySpeed;
  /** Current playhead in display time (post gap-compression). */
  viewT: number;
}

export interface ActivityTransportBounds {
  /** Scene display start. */
  d0: number;
  /** Scene display end — the live edge while the agent is working. */
  d1: number;
  /** Whether the underlying stream is still growing. */
  isLive: boolean;
}

const ACTIVITY_SPEEDS: readonly ActivitySpeed[] = [1, 2, 4, 8];
export const LIVE_REJOIN_THRESHOLD_MS = 150;
export const ACTIVITY_SCRUB_STEP_MS = 2_000;

function clampView(viewT: number, bounds: ActivityTransportBounds): number {
  return Math.min(Math.max(viewT, bounds.d0), bounds.d1);
}

export function createActivityTransport(
  bounds: ActivityTransportBounds,
): ActivityTransportState {
  if (bounds.isLive) {
    return { mode: "live", playing: true, speed: 1, viewT: bounds.d1 };
  }
  return { mode: "replay", playing: false, speed: 1, viewT: bounds.d1 };
}

/**
 * Advance the transport by one animation frame. In live mode the playhead
 * pins to the growing display end; in replay it advances by `dtMs * speed`
 * and either rejoins live at the edge or parks at the end of an idle
 * scope.
 */
export function tickActivityTransport(
  state: ActivityTransportState,
  dtMs: number,
  bounds: ActivityTransportBounds,
): ActivityTransportState {
  if (state.mode === "live") {
    if (!bounds.isLive) {
      // Scope went idle while we were watching live: park at the end.
      return { ...state, mode: "replay", playing: false, viewT: bounds.d1 };
    }
    return state.viewT === bounds.d1 ? state : { ...state, viewT: bounds.d1 };
  }

  if (!state.playing) {
    const clamped = clampView(state.viewT, bounds);
    return clamped === state.viewT ? state : { ...state, viewT: clamped };
  }

  const advanced = state.viewT + Math.max(0, dtMs) * state.speed;

  if (advanced >= bounds.d1 - LIVE_REJOIN_THRESHOLD_MS && bounds.isLive) {
    return { ...state, mode: "live", playing: true, viewT: bounds.d1 };
  }

  if (advanced >= bounds.d1) {
    return { ...state, playing: false, viewT: bounds.d1 };
  }

  return { ...state, viewT: clampView(advanced, bounds) };
}

/** Scrub to an absolute display time; always drops into replay mode. */
export function scrubActivityTransport(
  state: ActivityTransportState,
  viewT: number,
  bounds: ActivityTransportBounds,
): ActivityTransportState {
  return {
    ...state,
    mode: "replay",
    viewT: clampView(viewT, bounds),
  };
}

/** Relative scrub (e.g. ±2s keyboard jumps). */
export function jumpActivityTransport(
  state: ActivityTransportState,
  deltaMs: number,
  bounds: ActivityTransportBounds,
): ActivityTransportState {
  const from = state.mode === "live" ? bounds.d1 : state.viewT;
  const target = from + deltaMs;

  if (deltaMs > 0 && target >= bounds.d1) {
    if (bounds.isLive) {
      return { ...state, mode: "live", playing: true, viewT: bounds.d1 };
    }
    return { ...state, mode: "replay", viewT: bounds.d1 };
  }

  return scrubActivityTransport(state, target, bounds);
}

/**
 * Toggle play/pause. Pausing live drops into replay at the frozen playhead;
 * resuming at the end of an idle scope restarts from the beginning.
 */
export function toggleActivityPlayback(
  state: ActivityTransportState,
  bounds: ActivityTransportBounds,
): ActivityTransportState {
  if (state.mode === "live") {
    return { ...state, mode: "replay", playing: false, viewT: bounds.d1 };
  }

  if (state.playing) {
    return { ...state, playing: false };
  }

  if (!bounds.isLive && state.viewT >= bounds.d1) {
    return { ...state, playing: true, viewT: bounds.d0 };
  }

  return { ...state, playing: true };
}

/** Jump back to the live edge (no-op while the scope is idle). */
export function jumpToLiveActivityTransport(
  state: ActivityTransportState,
  bounds: ActivityTransportBounds,
): ActivityTransportState {
  if (!bounds.isLive) return state;
  return { ...state, mode: "live", playing: true, viewT: bounds.d1 };
}

/** Restart playback from the beginning of the scene. */
export function restartActivityTransport(
  state: ActivityTransportState,
  bounds: ActivityTransportBounds,
): ActivityTransportState {
  return { ...state, mode: "replay", playing: true, viewT: bounds.d0 };
}

/** Cycle 1× → 2× → 4× → 8× → 1×. */
export function cycleActivitySpeed(
  state: ActivityTransportState,
): ActivityTransportState {
  const index = ACTIVITY_SPEEDS.indexOf(state.speed);
  const next = ACTIVITY_SPEEDS[(index + 1) % ACTIVITY_SPEEDS.length];
  return { ...state, speed: next };
}

export type ActivityTransportBadge = "live" | "replay" | "idle";

/**
 * Which status badge the transport should show.
 *
 * "idle", not "ended": parked at the end of a currently-inactive scope
 * describes the PRESENT (nothing running, you are caught up), while "ended"
 * would claim the future — any new turn can spike the thread awake, at
 * which point the badge flips back to live.
 */
export function activityTransportBadge(
  state: ActivityTransportState,
  bounds: ActivityTransportBounds,
): ActivityTransportBadge {
  if (state.mode === "live") return "live";
  if (!bounds.isLive && state.viewT >= bounds.d1) return "idle";
  return "replay";
}
