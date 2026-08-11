export type IdentityHandoffCredential = {
  code: string;
  policyReceipt?: string;
};

// Identity handoff credentials must survive an identity replacement without
// becoming durable. The transaction id is non-secret and is the only key that
// leaves this module; a process restart abandons the Map and requires a fresh
// dashboard handoff.
const identityHandoffs = new Map<string, IdentityHandoffCredential>();

export function storeIdentityHandoff(
  transactionId: string,
  credential: IdentityHandoffCredential,
): void {
  identityHandoffs.set(transactionId, { ...credential });
}

export function getIdentityHandoff(
  transactionId: string,
): IdentityHandoffCredential | null {
  const credential = identityHandoffs.get(transactionId);
  return credential ? { ...credential } : null;
}

export function setIdentityHandoffPolicyReceipt(
  transactionId: string,
  policyReceipt: string,
): boolean {
  const credential = identityHandoffs.get(transactionId);
  if (!credential) return false;
  identityHandoffs.set(transactionId, { code: credential.code, policyReceipt });
  return true;
}

export function destroyIdentityHandoff(transactionId: string): void {
  identityHandoffs.delete(transactionId);
}

/** Simulates process teardown in tests and prevents test credentials leaking. */
export function clearIdentityHandoffVault(): void {
  identityHandoffs.clear();
}

export function identityHandoffVaultSize(): number {
  return identityHandoffs.size;
}
