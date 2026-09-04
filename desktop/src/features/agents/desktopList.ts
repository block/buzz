import { invoke } from "@tauri-apps/api/core";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";

export type DesktopRow = { id: string; name: string; updated: number };
export type DesktopScope = { owner: string; community: string };
export type DesktopList = {
  rows: DesktopRow[];
  local: string;
  partial: boolean;
  warning: string;
};
const KIND = 30180;
const LIMIT = 100;

/** Every continuation belongs to this mounted owner/community, including late ACKs. */
export async function refreshDesktopList(
  scope: DesktopScope,
  active: () => boolean,
  ipc = invoke,
  relay = relayClient,
): Promise<DesktopList> {
  const epoch = relay.getSessionEpoch();
  const check = () => {
    if (!active() || epoch !== relay.getSessionEpoch())
      throw new Error("Desktop scope changed");
  };
  const wait = async <T>(work: Promise<T>) => {
    const result = await work;
    check();
    return result;
  };
  const read = (events: RelayEvent[]) =>
    wait(ipc<DesktopRow[]>("read_desktop_profiles", { ...scope, events }));
  const filter = { kinds: [KIND], authors: [scope.owner] };
  check();
  let local = "";
  let warning = "";
  try {
    const { event } = await wait(
      ipc<{ event: RelayEvent }>("prepare_desktop_profile", scope),
    );
    const [profile] = await read([event]);
    local = profile.id;
    // A bounded inventory is never evidence that this coordinate is missing.
    const head = await wait(
      relay.fetchEvents({ ...filter, "#d": [local], limit: 1 }),
    );
    await read(head);
    if (head.length && head[0].id !== event.id)
      throw new Error("Desktop profile differs on relay");
    if (!head.length)
      await wait(
        relay.publishEvent(
          event,
          "Desktop publish timed out",
          "Desktop publish failed",
        ),
      );
  } catch {
    check();
    warning =
      "This Desktop profile could not be synchronized. Retry to publish it.";
  }
  // Listing is independent of local publication and never uses scalar presence.
  const events = await wait(relay.fetchEvents({ ...filter, limit: LIMIT }));
  const rows = await read(events);
  return { rows, local, partial: events.length === LIMIT, warning };
}
