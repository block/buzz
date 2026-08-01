/**
 * NIP-44 encrypt-to-self and NIP-AB device-pairing bridge calls.
 *
 * Split out of `tauri.ts` to keep that module under the desktop file-size
 * ceiling; see `desktop/scripts/check-file-sizes.mjs`.
 */
import { invokeTauri } from "./tauri";

// ── NIP-44 encrypt-to-self ───────────────────────────────────────────────────

export async function nip44EncryptToSelf(plaintext: string): Promise<string> {
  return invokeTauri<string>("nip44_encrypt_to_self", { plaintext });
}

export async function nip44DecryptFromSelf(
  ciphertext: string,
): Promise<string> {
  return invokeTauri<string>("nip44_decrypt_from_self", { ciphertext });
}

// ── NIP-AB device pairing ───────────────────────────────────────────────────

export async function startPairing(): Promise<string> {
  return invokeTauri<string>("start_pairing");
}

export async function confirmPairingSas(): Promise<void> {
  await invokeTauri("confirm_pairing_sas");
}

export async function cancelPairing(): Promise<void> {
  await invokeTauri("cancel_pairing");
}
