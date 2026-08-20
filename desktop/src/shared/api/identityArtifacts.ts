import type { RelayEvent } from "@/shared/api/types";
import { invokeTauri } from "./tauri";
import type { StampedArtifact } from "./stampedArtifact";

// Owner-identity artifact producers (NIP-AP). Each Rust command returns the
// owner-key-derived value wrapped as `{ value, artifact }`; C6/C7 threads the
// pair to each identity-sensitive application site and adds the
// generation-compare. Until then these adapters unwrap `.value` at the boundary
// — the stamp is inert but the wire shape is pinned (see `StampedArtifact`).

export async function signRelayEvent(input: {
  kind: number;
  content: string;
  createdAt?: number;
  tags: string[][];
}): Promise<RelayEvent> {
  // C6/C7: thread {value, artifact} to application sites — unwrap is temporary.
  const r = await invokeTauri<StampedArtifact<string>>("sign_event", input);
  return JSON.parse(r.value) as RelayEvent;
}

export async function createAuthEvent(input: {
  challenge: string;
  relayUrl: string;
}): Promise<RelayEvent> {
  // C6/C7: thread {value, artifact} to application sites — unwrap is temporary.
  const r = await invokeTauri<StampedArtifact<string>>(
    "create_auth_event",
    input,
  );
  return JSON.parse(r.value) as RelayEvent;
}

export async function nip44EncryptToSelf(plaintext: string): Promise<string> {
  // C6/C7: thread {value, artifact} to application sites — unwrap is temporary.
  const r = await invokeTauri<StampedArtifact<string>>(
    "nip44_encrypt_to_self",
    {
      plaintext,
    },
  );
  return r.value;
}

export async function nip44DecryptFromSelf(
  ciphertext: string,
): Promise<string> {
  // C6/C7: thread {value, artifact} to application sites — unwrap is temporary.
  const r = await invokeTauri<StampedArtifact<string>>(
    "nip44_decrypt_from_self",
    {
      ciphertext,
    },
  );
  return r.value;
}
