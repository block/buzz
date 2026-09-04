import { invoke } from "@tauri-apps/api/core";
import { useQuery } from "@tanstack/react-query";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import type { DesktopScope } from "./desktopList";

export type DesktopObservation = { id: string; heard: number };
export const DESKTOP_PULSE_MS = 60_000;

/** A failed pulse does not hide other Desktops; failed reads retain cached observations. */
export async function refreshDesktopObservations(
  scope: DesktopScope,
  active: () => boolean,
  ipc = invoke,
  relay = relayClient,
) {
  const epoch = relay.getSessionEpoch();
  const check = () => {
    if (!active() || epoch !== relay.getSessionEpoch())
      throw new Error("Desktop observation scope changed");
  };
  const wait = async <T>(work: Promise<T>) => {
    const result = await work;
    check();
    return result;
  };
  check();
  let warning = "";
  try {
    const { event } = await wait(
      ipc<{ event: RelayEvent }>("prepare_desktop_observation", scope),
    );
    await wait(
      relay.publishEvent(
        event,
        "Desktop pulse timed out",
        "Desktop pulse failed",
        check,
      ),
    );
  } catch {
    check();
    // The next bounded interval retries with a new observation, not an old heartbeat.
    warning = "This Desktop could not report its last-heard time. Will retry.";
  }
  const events = await wait(
    relay.fetchEvents({ kinds: [30181], authors: [scope.owner], limit: 100 }),
  );
  const rows = await wait(
    ipc<DesktopObservation[]>("read_desktop_observations", {
      ...scope,
      // History is chronological and may include a live replacement before EOSE.
      // First matching host wins in the view: newest signed time, then lower ID.
      events: [...events].sort(
        (a, b) =>
          b.created_at - a.created_at ||
          (a.id < b.id ? -1 : a.id > b.id ? 1 : 0),
      ),
    }),
  );
  return { rows, warning, partial: events.length === 100 };
}

export function useDesktopObservations(scope: DesktopScope | null) {
  return useQuery({
    queryKey: ["desktop-observations", scope?.owner, scope?.community],
    enabled: !!scope,
    queryFn: ({ signal }) => {
      if (!scope) throw new Error("Desktop scope unavailable");
      return refreshDesktopObservations(scope, () => !signal.aborted);
    },
    gcTime: 0,
    staleTime: DESKTOP_PULSE_MS / 2,
    retry: false,
    refetchOnWindowFocus: false,
  });
}

/** Signed sender time is advisory, including clock skew; it never establishes agent death. */
export function desktopFreshness(heard: number | undefined, now: number) {
  if (heard === undefined) return "Unknown";
  const state =
    heard > now
      ? "Unknown (Desktop clock ahead)"
      : now - heard <= 180
        ? "Recent"
        : "Stale";
  return `${state} · ${new Date(heard * 1000).toLocaleString()}`;
}
