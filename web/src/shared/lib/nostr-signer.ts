import {
  finalizeEvent,
  generateSecretKey,
  getPublicKey,
} from "nostr-tools/pure";

export type UnsignedNostrEvent = {
  kind: number;
  created_at: number;
  tags: string[][];
  content: string;
};

export type SignedNostrEvent = UnsignedNostrEvent & {
  id: string;
  pubkey: string;
  sig: string;
};

type Nip07Provider = {
  getPublicKey(): Promise<string>;
  signEvent(event: UnsignedNostrEvent): Promise<SignedNostrEvent>;
};

declare global {
  interface Window {
    nostr?: Nip07Provider;
  }
}

export class Nip07UnavailableError extends Error {
  constructor() {
    super("A NIP-07 browser extension is required to join in the browser.");
    this.name = "Nip07UnavailableError";
  }
}

let ephemeralSecretKey: Uint8Array | null = null;
let enterpriseSigner: Pick<Nip07Provider, "signEvent"> | null = null;
let signerGeneration = 0;

/** Install before connecting. Null explicitly selects self-custody, not enterprise logout. Stores no credentials. */
export function configureEnterpriseSigner(
  signer: Pick<Nip07Provider, "signEvent"> | null,
): void {
  signerGeneration++;
  enterpriseSigner = signer;
  ephemeralSecretKey?.fill(0);
  ephemeralSecretKey = null;
}

/** Durable membership may use either corporate custody or a user-owned NIP-07 identity. */
export function hasDurableSigner(): boolean {
  return enterpriseSigner !== null || hasNip07Provider();
}

function getEphemeralSecretKey(): Uint8Array {
  if (!ephemeralSecretKey) {
    ephemeralSecretKey = generateSecretKey();
  }
  return ephemeralSecretKey;
}

export function hasNip07Provider(): boolean {
  return typeof window !== "undefined" && window.nostr != null;
}

function sameUnsignedEvent(
  expected: UnsignedNostrEvent,
  actual: SignedNostrEvent,
): boolean {
  return (
    actual.kind === expected.kind &&
    actual.created_at === expected.created_at &&
    actual.content === expected.content &&
    JSON.stringify(actual.tags) === JSON.stringify(expected.tags)
  );
}

/**
 * Sign with NIP-07 when available, otherwise use a page-lifetime key.
 *
 * The ephemeral fallback preserves anonymous browsing on open relays. Flows
 * that create durable membership must set `requireNip07` so a reload cannot
 * orphan a relay-membership row.
 */
export async function signNostrEvent(
  template: Omit<UnsignedNostrEvent, "created_at"> & {
    created_at?: number;
  },
  options?: { requireNip07?: boolean },
): Promise<SignedNostrEvent> {
  const unsigned: UnsignedNostrEvent = {
    ...template,
    created_at: template.created_at ?? Math.floor(Date.now() / 1000),
  };
  const provider = typeof window === "undefined" ? undefined : window.nostr;

  if (enterpriseSigner) {
    const generation = signerGeneration;
    const signed = await enterpriseSigner.signEvent(unsigned);
    if (generation !== signerGeneration) {
      throw new Error("Enterprise identity changed while signing.");
    }
    return signed;
  }

  if (provider) {
    const expectedPubkey = await provider.getPublicKey();
    const signed = await provider.signEvent(unsigned);
    if (
      signed.pubkey !== expectedPubkey ||
      !sameUnsignedEvent(unsigned, signed) ||
      typeof signed.id !== "string" ||
      typeof signed.sig !== "string"
    ) {
      throw new Error("The NIP-07 extension returned an invalid signed event.");
    }
    return signed;
  }

  if (options?.requireNip07) {
    throw new Nip07UnavailableError();
  }

  const secretKey = getEphemeralSecretKey();
  const signed = finalizeEvent(unsigned, secretKey);
  if (signed.pubkey !== getPublicKey(secretKey)) {
    throw new Error("Failed to create the ephemeral browser identity.");
  }
  return signed;
}
