export type ProjectConnectionScope = {
  relayUrl: string;
  operatorPubkey: string;
  /** Durable NIP-34 repository coordinate. Local Project.id is never persisted. */
  repoAddress: string;
};
