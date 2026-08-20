import { invokeTauri } from "@/shared/api/tauri";

/**
 * Read the active relay's NIP-11 `self` pubkey (its own signing key, hex), or
 * `null` when the relay advertises none or an invalid key. Network and malformed
 * document failures reject the request. Callers that use this value to trust
 * relay-signed state must treat both `null` and errors as untrusted.
 */
export function getRelaySelf(): Promise<string | null> {
  return invokeTauri<string | null>("get_relay_self");
}
