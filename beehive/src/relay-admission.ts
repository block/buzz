/** Admission proof supplied by a reviewed integration, never loaded from user JSON.
 * Owner-approved route: ordinary direct relay membership, without NIP-OA.
 * Registration alone or successful open-relay AUTH is not membership evidence.
 */
export type ScopedRelayAdmission = {
  admit(scope: { relay: string; publicKey: string; transport: 'nip42-nip59'; ownerDelegation: false }): void;
};
/** No production admission profile is established. Do not dial or send NIP-OA. */
export const productionAdmission: ScopedRelayAdmission = {
  admit() { throw Error('Host relay admission pending: verified ordinary direct membership required (relay admin: buzz-admin add-member --pubkey <host-public-key> --role member); NIP-OA prohibited'); },
};
