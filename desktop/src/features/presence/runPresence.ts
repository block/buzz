/** Relay-bounded leases; snapshot reads never renew these deadlines. */
export type PresenceRun = {
  run: string;
  seq: number;
  status: "online" | "away" | "offline";
  expires_at: number;
  location: { host: string; label: string } | null;
  registration: string | null;
};
export type PresenceRuns = Record<string, PresenceRun[]>;

export function activeRuns(runs: PresenceRun[] | undefined, now: number) {
  return (runs ?? []).filter(
    (run) => run.status !== "offline" && run.expires_at > now,
  );
}

export function locationLabels(runs: PresenceRun[] | undefined, now: number) {
  return [
    ...new Set(
      activeRuns(runs, now).flatMap((run) =>
        run.location ? [run.location.label] : [],
      ),
    ),
  ].sort();
}

export function nextRunExpiry(data: PresenceRuns | undefined, now: number) {
  const deadlines = Object.values(data ?? {})
    .flat()
    .map((run) => run.expires_at)
    .filter((deadline) => deadline > now);
  return deadlines.length ? Math.min(...deadlines) : undefined;
}
