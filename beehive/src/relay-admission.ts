/** Admission proof supplied by a reviewed integration, never loaded from user JSON.
 * Registration and ordinary relay membership do not establish this contract.
 */
export type ScopedRelayAdmission = {
  admit(scope: { relay: string; publicKey: string; transport: 'nip42-nip59'; ownerDelegation: false }): void;
};
/** No production admission profile is established. Do not dial or send NIP-OA. */
export const productionAdmission: ScopedRelayAdmission = {
  admit() { throw Error('Host relay admission pending: reviewed scoped infrastructure permissions required; NIP-OA prohibited'); },
};
