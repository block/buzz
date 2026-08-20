/**
 * The `{ value, artifact }` pair the owner-identity artifact producers return
 * across the Tauri boundary. `artifact` is the generation stamp
 * (`OwnerIdentityCapability<ArtifactPolicy>`, NIP-AP): `id` is
 * correlation/diagnostic only, `generation` is the identity-persistence
 * generation the value was signed/decrypted under.
 *
 * The wire contract is locked here and on the Rust side (`StampedArtifact<T>`):
 * C6/C7 threads this pair to each identity-sensitive application site and adds
 * the generation-compare (refuse to apply once the current generation advances
 * past `artifact.generation`). Until then the adapters unwrap `.value` at the
 * boundary — the stamp is inert but the shape is pinned so C6/C7 inherits a
 * stable contract, not a rediscovery.
 */
export interface StampedArtifact<T> {
  value: T;
  artifact: { id: number; generation: number };
}
