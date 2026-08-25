export const BUSINESS_AUTH_HEARTBEAT_INTERVAL_MS = 15_000;

type IntervalScheduler = {
  setInterval(
    callback: () => void,
    delay: number,
  ): ReturnType<typeof setInterval>;
  clearInterval(id: ReturnType<typeof setInterval>): void;
};

export function startBusinessAuthHeartbeat(
  enabled: boolean,
  check: () => void,
  scheduler: IntervalScheduler = globalThis,
): () => void {
  if (!enabled) return () => undefined;
  const interval = scheduler.setInterval(
    check,
    BUSINESS_AUTH_HEARTBEAT_INTERVAL_MS,
  );
  return () => scheduler.clearInterval(interval);
}
