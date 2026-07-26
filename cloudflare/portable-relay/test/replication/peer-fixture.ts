// Deterministic test peer shared between the vitest replication config
// (which bakes its pubkey into BUZZ_REPLICATION_PEERS) and the tests
// (which sign NIP-98 evidence with the secret). Test-only material.

export const TEST_REPLICATION_SOURCE = "laptop-test/sovereign";
export const TEST_PEER_PRINCIPAL = "did:example:buzz-laptop-test";
export const TEST_PEER_SECRET_HEX =
  "1111111111111111111111111111111111111111111111111111111111111111";

export function hexToBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let index = 0; index < bytes.length; index += 1) {
    bytes[index] = Number.parseInt(hex.slice(index * 2, index * 2 + 2), 16);
  }
  return bytes;
}
