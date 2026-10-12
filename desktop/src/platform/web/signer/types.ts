// Pluggable web signer. The page NEVER holds the secret key; all signing is delegated.
export type UnsignedEvent = {
  kind: number;
  content: string;
  tags: string[][];
  created_at?: number;
};
export type SignedEvent = UnsignedEvent & {
  id: string;
  pubkey: string;
  sig: string;
  created_at: number;
};

/** 0 = user-initiated send/write, 1 = relay AUTH / read-critical (NIP-98), 2 = profile/settings/crypto, 3 = presence/telemetry (droppable). */
export type SignPriority = 0 | 1 | 2 | 3;
export type SignOpts = { priority?: SignPriority };

export type SignerKind = "nip07" | "nip46";

export interface Signer {
  readonly kind: SignerKind;
  getPublicKey(): Promise<string>;
  /** `opts.priority` is a scheduling hint for the instrumented wrapper (0 user action … 3 background); raw signers ignore it. */
  signEvent(event: UnsignedEvent, opts?: SignOpts): Promise<SignedEvent>;
  /** Optional NIP-44 support (extension may not offer it). */
  readonly nip44?: {
    encrypt(peerPubkey: string, plaintext: string): Promise<string>;
    decrypt(peerPubkey: string, ciphertext: string): Promise<string>;
  };
}

export class SignerUnavailableError extends Error {
  constructor(
    public reason: "no-extension" | "nip46-not-implemented" | "no-session",
    message: string,
  ) {
    super(message);
    this.name = "SignerUnavailableError";
  }
}
