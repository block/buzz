import { invokeTauri } from "@/shared/api/tauri";

export async function startIdentityRecoveryPairing(): Promise<string> {
  return invokeTauri<string>("start_identity_recovery_pairing");
}
