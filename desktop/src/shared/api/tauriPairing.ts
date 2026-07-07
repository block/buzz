import { invokeTauri } from "@/shared/api/tauri";

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
