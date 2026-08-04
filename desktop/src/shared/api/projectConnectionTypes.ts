export type ProjectConnectionScope = {
  relayUrl: string;
  operatorPubkey: string;
  /**
   * Durable NIP-MP Project coordinate. Legacy one-repository Projects use their
   * NIP-34 repository coordinate.
   */
  projectAddress: string;
};
