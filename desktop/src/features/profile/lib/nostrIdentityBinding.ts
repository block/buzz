import { invokeTauri } from "@/shared/api/tauri";

export type NostrIdentityBindingInput = {
  challengeId: string;
  nonce: string;
  origin: string;
  expiresAt: string;
};

export function signNostrIdentityBinding(
  input: NostrIdentityBindingInput,
): Promise<string> {
  return invokeTauri<string>("sign_nostr_identity_binding", input);
}
