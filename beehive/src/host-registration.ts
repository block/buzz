import { schnorr } from '@noble/curves/secp256k1';
import { digest, fields, object, publicKey, text } from './protocol.ts';

/** Offline pairing request. This is private file-exchange material, not a relay event. */
export type HostPairing = { version: 1; purpose: 'beehive-host-registration'; host: string; owner: string; label: string; relay: string; nonce: string };
/** Owner endorsement of infrastructure only; never a NIP-OA tag or relay admission. */
export type HostRegistration = { request: HostPairing; expires: number; signature: string };
/** Validate untrusted pairing files before displaying or signing them. */
export function hostPairing(value: unknown): HostPairing {
  const p = object(value);
  fields(p, ['version', 'purpose', 'host', 'owner', 'label', 'relay', 'nonce']);
  if (p.version !== 1 || p.purpose !== 'beehive-host-registration') throw Error('Invalid registration purpose');
  for (const key of ['host', 'owner', 'nonce']) if (typeof p[key] !== 'string' || !/^[0-9a-f]{64}$/.test(p[key] as string)) throw Error('Invalid pairing identity/nonce');
  if (p.host === p.owner) throw Error('Host must have independent identity');
  const label = text(p.label);
  if (label.length > 128 || /[\x00-\x1f\x7f]/.test(label)) throw Error('Invalid host label');
  const relay = text(p.relay); const url = new URL(relay);
  if (relay.length > 2048 || url.username || url.password || url.hash || url.search || !(url.protocol === 'wss:' || (url.protocol === 'ws:' && url.hostname === '127.0.0.1'))) throw Error('Invalid pairing relay');
  return { version: 1, purpose: 'beehive-host-registration', host: p.host as string, owner: p.owner as string, label, relay, nonce: p.nonce as string };
}
/** Full fingerprint binds host, owner, label, relay and this installation request. */
export function pairingFingerprint(value: unknown): string {
  return digest(JSON.stringify(hostPairing(value))).toString('hex');
}
function preimage(request: HostPairing, expires: number): Buffer {
  if (!Number.isSafeInteger(expires) || expires <= 0) throw Error('Invalid registration expiration');
  return digest(`beehive:host-registration:v1:${pairingFingerprint(request)}:${expires}`);
}
/** Explicit existing-owner signing; the caller must not persist the owner secret. */
export function registerHost(request: unknown, ownerSecret: string, expires: number): HostRegistration {
  const pairing = hostPairing(request);
  if (pairing.owner !== publicKey(ownerSecret)) throw Error('Wrong owner signer');
  return { request: pairing, expires, signature: Buffer.from(schnorr.sign(preimage(pairing, expires), ownerSecret)).toString('hex') };
}
/** Verifies only registration and locally observed expiration, not remote revocation or admission. */
export function verifyHostRegistration(value: unknown, expected: unknown, now = Math.floor(Date.now() / 1000)): HostRegistration {
  const r = object(value); fields(r, ['request', 'expires', 'signature']);
  const request = hostPairing(r.request);
  if (pairingFingerprint(request) !== pairingFingerprint(expected)) throw Error('Registration does not match pairing request');
  if (typeof r.expires !== 'number' || !Number.isSafeInteger(now) || now >= r.expires || typeof r.signature !== 'string' || !/^[0-9a-f]{128}$/.test(r.signature)) throw Error('Invalid/expired host registration');
  if (!schnorr.verify(r.signature, preimage(request, r.expires), request.owner)) throw Error('Invalid owner registration signature');
  return { request, expires: r.expires, signature: r.signature };
}
