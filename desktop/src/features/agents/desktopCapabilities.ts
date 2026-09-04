import { invoke } from "@tauri-apps/api/core";
import { useQuery } from "@tanstack/react-query";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import type { DesktopScope } from "./desktopList";

export type DesktopCapabilities = {
  id: string;
  reported: number;
  runtimes: {
    id: string;
    availability: string;
    requires_external_cli: boolean;
    max_parallelism: number | null;
  }[];
};

/** Exact signed reports are persisted natively; only changed facts create new bytes. */
export async function refreshDesktopCapabilities(
  scope: DesktopScope,
  active: () => boolean,
  ipc = invoke,
  relay = relayClient,
) {
  const epoch = relay.getSessionEpoch();
  const check = () => {
    if (!active() || epoch !== relay.getSessionEpoch())
      throw new Error("Desktop capability scope changed");
  };
  const wait = async <T>(work: Promise<T>) => {
    const result = await work;
    check();
    return result;
  };
  const read = (events: RelayEvent[]) =>
    wait(
      ipc<DesktopCapabilities[]>("read_desktop_capabilities", {
        ...scope,
        events,
      }),
    );
  const filter = { kinds: [30182], authors: [scope.owner] };
  check();
  let warning = "";
  try {
    const { event } = await wait(
      ipc<{ event: RelayEvent }>("prepare_desktop_capabilities", scope),
    );
    const [local] = await read([event]);
    const head = await wait(
      relay.fetchEvents({ ...filter, "#d": [local.id], limit: 1 }),
    );
    await read(head);
    if (!head.some((e) => e.id === event.id))
      await wait(
        relay.publishEvent(
          event,
          "Desktop report timed out",
          "Desktop report failed",
          check,
        ),
      );
  } catch {
    check();
    warning =
      "This Desktop could not synchronize capability facts. Will retry.";
  }
  const events = await wait(relay.fetchEvents({ ...filter, limit: 100 }));
  return { rows: await read(events), partial: events.length === 100, warning };
}

export function useDesktopCapabilities(scope: DesktopScope | null) {
  return useQuery({
    queryKey: ["desktop-capabilities", scope?.owner, scope?.community],
    enabled: !!scope,
    queryFn: ({ signal }) => {
      if (!scope) throw new Error("Desktop scope unavailable");
      return refreshDesktopCapabilities(scope, () => !signal.aborted);
    },
    gcTime: 0,
    staleTime: 30_000,
    retry: false,
    refetchOnWindowFocus: false,
  });
}
