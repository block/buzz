import { invokeTauri, type StampedArtifact } from "@/shared/api/tauri";

export type NostrIdentityBindingInput = {
  challengeId: string;
  nonce: string;
  verificationCode: string;
  origin: string;
  expiresAt: string;
};

export async function signNostrIdentityBinding(
  input: NostrIdentityBindingInput,
): Promise<string> {
  // C6/C7: thread {value, artifact} to application sites — unwrap is temporary.
  const { value } = await invokeTauri<StampedArtifact<string>>(
    "sign_nostr_identity_binding",
    input,
  );
  return value;
}
