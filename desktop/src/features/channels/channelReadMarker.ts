import { isPlausibleReadMarker } from "./readState/readStateFormat";

function parseTimestamp(value: string | null | undefined) {
  if (!value) {
    return null;
  }

  const timestamp = Date.parse(value);
  return Number.isNaN(timestamp) ? null : timestamp;
}

function toUnixSeconds(isoOrMs: string | null | undefined): number | null {
  const ms = parseTimestamp(isoOrMs);
  return ms === null ? null : Math.floor(ms / 1_000);
}

// Resolve where the read marker should land when a channel is marked read,
// from timestamps already in unix seconds. Folds the caller's timeline position
// together with the newest event this client has observed live
// (`observedLatest`), so an explicit "mark read" still covers messages that
// arrived faster than channel metadata — this fold is load-bearing for the Esc
// shortcut, sidebar mark-read, empty-channel open and mark-all-read, which can pass a null/stale caller value. `clearObserved` reports whether the
// resulting marker covers the observed timestamp, signalling the caller to drop
// its observed refs so the unread memo sees `latest === undefined` until a
// genuinely newer event arrives.
//
// Inputs may be event-derived, so `isPlausibleReadMarker`
// decides for each of them whether it could have come from a clock we agree
// with. An implausible input is *discarded*, not clamped: clamping it to the
// tolerance ceiling would manufacture a read frontier at `now + 120` and hide
// every legitimate message for the next two minutes. If no input survives, the
// marker is repaired to the present — the mark-read gesture is real, so it
// still takes effect, and only the future-dated event itself stays unread.
//
// `nowSeconds` is injectable so the policy is testable; it must stay a
// parameter rather than a captured constant.
export function resolveChannelReadMarkerUnix(
  callerUnix: number | null,
  observedLatest: number | undefined,
  nowSeconds: number = Date.now() / 1_000,
): { markAt: number | null; clearObserved: boolean } {
  const requested = Math.max(callerUnix ?? 0, observedLatest ?? 0) || null;
  if (requested === null) return { markAt: null, clearObserved: false };

  const now = Math.floor(nowSeconds);
  const plausible = [callerUnix, observedLatest].filter(
    (value): value is number =>
      value !== null &&
      value !== undefined &&
      value > 0 &&
      isPlausibleReadMarker(value, now),
  );
  const markAt = plausible.length > 0 ? Math.max(...plausible) : now;
  return {
    markAt,
    clearObserved:
      observedLatest !== undefined &&
      // A repaired marker does not cover a future-dated observed event, so the
      // observed refs must survive: that event really is still unread.
      observedLatest <= markAt,
  };
}

// String-timestamp front door for `resolveChannelReadMarkerUnix`, for the
// callers that hold a message's ISO `created_at`.
export function resolveChannelReadMarker(
  callerReadAt: string | null | undefined,
  observedLatest: number | undefined,
  nowSeconds: number = Date.now() / 1_000,
): { markAt: number | null; clearObserved: boolean } {
  return resolveChannelReadMarkerUnix(
    toUnixSeconds(callerReadAt),
    observedLatest,
    nowSeconds,
  );
}
