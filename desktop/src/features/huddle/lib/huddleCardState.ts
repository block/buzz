// Matches the ephemeral channel TTL used when a huddle is created.
export const HUDDLE_JOINABLE_WINDOW_SECONDS = 60 * 60;

export function isHuddleStartStale(
  createdAtSeconds: number,
  nowMs = Date.now(),
) {
  return nowMs / 1000 - createdAtSeconds > HUDDLE_JOINABLE_WINDOW_SECONDS;
}
