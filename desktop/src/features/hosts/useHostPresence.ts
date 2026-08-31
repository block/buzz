import { useEffect } from "react";
import { ReadOnlyRelayClient } from "@/shared/api/readOnlyRelayClient";
import { invokeTauri } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";

/** Launcher reachability is independent of human idle/away and profile refreshes. */
export function useHostPresence(
  owner: string | undefined,
  relayUrl: string | undefined,
  registration: RelayEvent | undefined,
) {
  const binding = registration ? JSON.stringify(registration) : undefined;
  useEffect(() => {
    if (!owner || !relayUrl || !binding) return;
    const registration = JSON.parse(binding) as RelayEvent;
    let active = true;
    let seq = 0;
    let pending = false;
    const run = crypto.randomUUID().replaceAll("-", "");
    const client = new ReadOnlyRelayClient(relayUrl);
    const pulse = async () => {
      if (!active || pending) return;
      pending = true;
      try {
        const event = await invokeTauri<RelayEvent>("create_host_presence", {
          expectedOwner: owner,
          registration,
          run,
          seq: seq++,
          status: "online",
        });
        if (active) await client.publishEvent(event);
      } catch {
        // No optimistic liveness. Retry next heartbeat; prior leases expire.
        client.disconnect();
      } finally {
        pending = false;
      }
    };
    void pulse();
    const timer = window.setInterval(() => void pulse(), 60_000);
    const wake = () => void pulse();
    window.addEventListener("online", wake);
    window.addEventListener("focus", wake);
    return () => {
      active = false;
      window.clearInterval(timer);
      window.removeEventListener("online", wake);
      window.removeEventListener("focus", wake);
      client.disconnect();
      // Best effort on unmount; identity switches are rejected natively, then TTL wins.
      const shutdown = new ReadOnlyRelayClient(relayUrl);
      void invokeTauri<RelayEvent>("create_host_presence", {
        expectedOwner: owner,
        registration,
        run,
        seq: seq++,
        status: "offline",
      })
        .then((event) => shutdown.publishEvent(event))
        .catch(() => {})
        .finally(() => shutdown.disconnect());
    };
    // The signed binding is immutable; refreshed snapshot objects must not restart a run.
  }, [owner, relayUrl, binding]);
}
