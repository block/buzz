import { useCallback, useState } from "react";
import { runtime } from "@/shared/runtime/client";

/**
 * Which community this client is connected to.
 *
 * A fact, like identity: one relay URL, no policy. Community *switching* — the
 * teardown ordering and reset sequence — is a capability, and does not live here.
 */
export async function readRelayUrl(): Promise<string> {
  return runtime.relayUrl();
}

/** The same fact for a component that wants it in render. */
export function useCommunity() {
  const [relayUrl, setRelayUrl] = useState("");

  const load = useCallback(async () => {
    const next = await readRelayUrl();
    setRelayUrl(next);
    return next;
  }, []);

  return { relayUrl, load };
}

/**
 * The storage key for anything scoped to one person in one community. Derived in
 * one place so a second caller cannot invent a different spelling of the same
 * scope.
 */
export function communityScope(relayUrl: string, pubkey: string) {
  return `${relayUrl}:${pubkey}`;
}
