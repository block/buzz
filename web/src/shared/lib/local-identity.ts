/**
 * Persistent browser-local identity — the join-by-address key.
 *
 * Why this exists: `requireNip07` guards flows that create durable relay
 * membership because the previous fallback key was page-lifetime — claiming
 * with it would strand the membership on reload. A phone with no extension
 * therefore could not join at all, which rules out the one-machine invite.
 * A PERSISTENT local key closes that gap: it survives reloads, so the
 * membership it claims stays reachable from the same browser.
 *
 * Honesty contract, rendered by every surface that uses it:
 * - the key lives in THIS browser's localStorage — nothing else backs it up;
 * - the secret is exportable (nsec) and the UI must say so plainly;
 * - a NIP-07 provider, when present, is always preferred over this key.
 */

import { npubEncode, nsecEncode } from "nostr-tools/nip19";
import {
  finalizeEvent,
  generateSecretKey,
  getPublicKey,
} from "nostr-tools/pure";
import type { SignedNostrEvent, UnsignedNostrEvent } from "./nostr-signer";

const STORAGE_KEY = "buzz.local-identity.v1";

type StoredIdentity = { v: 1; secret: number[] };

function readStoredSecret(): Uint8Array | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as StoredIdentity;
    if (parsed?.v !== 1 || !Array.isArray(parsed.secret)) return null;
    const bytes = new Uint8Array(parsed.secret);
    if (bytes.length !== 32) return null;
    return bytes;
  } catch {
    return null;
  }
}

function writeStoredSecret(secret: Uint8Array): void {
  const value: StoredIdentity = { v: 1, secret: Array.from(secret) };
  localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
}

export type LocalKeypair = {
  secretKey: Uint8Array;
  pubkey: string;
  npub: string;
};

/** Get (or create once, persistently) this browser's local identity. */
export function getLocalKeypair(): LocalKeypair {
  let secret = readStoredSecret();
  if (!secret) {
    secret = generateSecretKey();
    writeStoredSecret(secret);
  }
  return {
    secretKey: secret,
    pubkey: getPublicKey(secret),
    npub: npubEncode(getPublicKey(secret)),
  };
}

/** Export the local identity secret as nsec — the user's only copy-out. */
export function exportLocalNsec(): string {
  return nsecEncode(getLocalKeypair().secretKey);
}

/** Forget the local identity. Lossy on purpose: the next call makes a new key. */
export function resetLocalIdentity(): void {
  localStorage.removeItem(STORAGE_KEY);
}

/**
 * Sign an event with the persistent local key — the signer passed to
 * join-by-address flows when no NIP-07 provider is present.
 */
export function signWithLocalKey(
  unsigned: UnsignedNostrEvent,
): SignedNostrEvent {
  const { secretKey } = getLocalKeypair();
  const signed = finalizeEvent(unsigned, secretKey);
  if (signed.pubkey !== getPublicKey(secretKey)) {
    throw new Error("Failed to sign with the local browser identity.");
  }
  return signed;
}
