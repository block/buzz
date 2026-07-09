import { invokeTauri } from "@/shared/api/tauri";

/**
 * Read the active relay's NIP-11 `self` pubkey (its own signing key, hex), or
 * `null` when the relay advertises none / is unreachable / serves a malformed
 * document. Used by the moderation UI to recognize a DM with the relay identity
 * (a moderation DM). Fails open by contract: a `null` result must be treated as
 * "not the relay", never as an error.
 */
export function getRelaySelf(): Promise<string | null> {
  return invokeTauri<string | null>("get_relay_self");
}
