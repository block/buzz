/** Human-readable Drive chrome activity captions from structured events. */

export type DriveActivityHit = {
  tag?: string;
  role?: string;
  name?: string;
};

export type DriveActivityPayload = {
  kind?: string;
  ok?: boolean;
  hit?: DriveActivityHit | null;
  url?: string;
  key?: string;
  text?: string;
  x?: number;
  y?: number;
  error?: string;
};

/**
 * Sustained quiet since last Drive/snapshot activity before "Finished".
 * Long enough to cover LLM think-time between browser_snapshot / browser_drive
 * calls so the strip stays one continuous turn (~12–15s tunable).
 */
export const DRIVE_ACTIVITY_TURN_IDLE_MS = 14_000;
/** How long "Finished" stays before the strip fades/clears. */
export const DRIVE_ACTIVITY_CLEAR_MS = 1_800;

/** @deprecated Use DRIVE_ACTIVITY_TURN_IDLE_MS — kept briefly for any stale imports. */
export const DRIVE_ACTIVITY_IDLE_MS = DRIVE_ACTIVITY_TURN_IDLE_MS;
/** @deprecated Use DRIVE_ACTIVITY_CLEAR_MS */
export const DRIVE_ACTIVITY_FINISHED_MS = DRIVE_ACTIVITY_CLEAR_MS;

export const DRIVE_ACTIVITY_OPENER = "Driving…";
export const DRIVE_ACTIVITY_FINISHED = "Finished";

function truncate(value: string, max: number): string {
  const trimmed = value.trim();
  if (trimmed.length <= max) return trimmed;
  return `${trimmed.slice(0, Math.max(0, max - 1))}…`;
}

function shortUrl(url: string | undefined): string {
  if (!url?.trim()) return "page";
  try {
    const parsed = new URL(url);
    const host = parsed.host || parsed.hostname;
    if (host) return host;
  } catch {
    // fall through
  }
  return truncate(url, 40);
}

function hitLabel(
  payload: DriveActivityPayload | null | undefined,
): string {
  const hit = payload?.hit;
  const name = hit?.name?.trim();
  if (name) return truncate(name, 32);
  const role = hit?.role?.trim();
  if (role) return role;
  const tag = hit?.tag?.trim();
  if (tag) return `<${tag}>`;
  if (
    typeof payload?.x === "number" &&
    typeof payload?.y === "number" &&
    Number.isFinite(payload.x) &&
    Number.isFinite(payload.y)
  ) {
    return `(${Math.round(payload.x)}, ${Math.round(payload.y)})`;
  }
  return "target";
}

/**
 * Map observe/drive event kinds + payload → chrome caption.
 * Returns null when the event should not update the activity strip.
 */
export function driveActivityCaption(
  eventKind: string,
  payload?: DriveActivityPayload | null,
): string | null {
  const kind = eventKind.trim().toLowerCase();
  if (kind === "drive_started") return DRIVE_ACTIVITY_OPENER;
  if (kind === "snapshot") return "Taking snapshot…";
  if (kind === "drive_error") return "Action failed";
  if (kind !== "drive") return null;

  const action = (payload?.kind ?? "").trim().toLowerCase();
  switch (action) {
    case "navigate":
      return `Opening ${shortUrl(payload?.url)}…`;
    case "click":
    case "clickat":
      return `Clicking ${hitLabel(payload)}…`;
    case "hover":
      return `Moving to ${hitLabel(payload)}…`;
    case "type":
    case "fill":
      // Never echo typed text — may be passwords / secrets.
      return "Typing…";
    case "key": {
      const key = payload?.key?.trim();
      return key ? `Pressing ${key}` : "Pressing key…";
    }
    case "scroll":
      return "Scrolling…";
    case "waitfor":
      return "Waiting…";
    default:
      return DRIVE_ACTIVITY_OPENER;
  }
}
