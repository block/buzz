import { invokeTauri } from "@/shared/api/tauri";

/**
 * Non-secret credential-persistence attestation for one managed agent
 * (`buzz.desktop.exact_agent_credential_persistence.v1`).
 *
 * The wire shape is intentionally snake_case and kept verbatim: the object is
 * an interop document whose `attestation_hash` binds the exact serialized
 * payload, so remapping field names client-side would break external
 * verification. Consumers treat unknown `persistence_backend` strings as
 * "not the backend I require".
 */
export type AgentPersistenceAttestation = {
  schema_version: string;
  agent_pubkey: string;
  persistence_backend: "os_keyring" | "inline_file";
  inline_fallback: boolean;
  parallelism: number;
  public_identity_hash: string;
  attestation_hash: string;
  stock_release_id: string;
  issued_at: string;
};

/**
 * Fetch the persistence attestation for a managed agent. Fails (rejects) with
 * `attestation_keyring_unreachable` or `attestation_credential_missing` when
 * persistence cannot be proven — callers must treat that as "not attested",
 * never as os_keyring.
 */
export async function getAgentPersistenceAttestation(
  pubkey: string,
): Promise<AgentPersistenceAttestation> {
  return invokeTauri<AgentPersistenceAttestation>(
    "get_agent_persistence_attestation",
    { pubkey },
  );
}
