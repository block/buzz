import { randomUUID } from 'node:crypto';
import { schnorr } from '@noble/curves/secp256k1';
import { finalizeEvent, verifyEvent, type Event } from 'nostr-tools/pure';
import { digest, fields, object, publicKey } from './protocol.ts';

/** Public identity only. Never derive about from a behavior profile. Empty fields clear. */
export type PublicMetadata = { display_name: string; picture: string; about: string };
/** Strict public-field allowlist; unknown/private fields fail closed. */
export function publicMetadata(input: unknown): PublicMetadata {
  const v = object(input); fields(v, ['display_name', 'picture', 'about']);
  for (const [key, limit] of [['display_name', 128], ['picture', 2048], ['about', 280]] as const) {
    if (typeof v[key] !== 'string' || [...v[key]].length > limit || /[\x00-\x1f\x7f]/.test(v[key])) throw Error('Invalid public metadata');
  }
  if (v.picture) { const u = new URL(String(v.picture)); if (u.protocol !== 'https:' || u.username || u.password) throw Error('Picture must be a public HTTPS URL'); }
  return { display_name: String(v.display_name), picture: String(v.picture), about: String(v.about) };
}
/** Canonical desired-content hash, independent of host placement and instructions. */
export function metadataRevision(value: PublicMetadata): string { return digest(JSON.stringify(publicMetadata(value))).toString('hex'); }
/** Native NIP-OA preimage/conditions from buzz-sdk nip_oa.rs at 051c3a270.
 * Consume an existing association only; never manufacture owner authorization.
 * Check both kind0 and request kind27235 against its conditions.
 */
export function metadataAuthorization(raw: string, agent: string, owner: string, now: number): string[] {
  const a: unknown = JSON.parse(raw);
  if (!Array.isArray(a) || a.length !== 4 || !a.every(v => typeof v === 'string') || a[0] !== 'auth' || a[1] !== owner || owner === agent || !/^[a-f0-9]{64}$/.test(owner) || !/^[a-f0-9]{128}$/.test(a[3])) throw Error('Invalid existing agent-owner association');
  const conditions = a[2] as string;
  for (const clause of conditions ? conditions.split('&') : []) {
    const match = /^(kind=|created_at<|created_at>)(0|[1-9][0-9]*)$/.exec(clause);
    if (!match) throw Error('Unsupported owner association conditions');
    const n = Number(match[2]);
    if (!Number.isSafeInteger(n) || n > (match[1] === 'kind=' ? 65535 : 4294967295)) throw Error('Invalid owner association conditions');
    if (match[1] === 'kind=' || (match[1] === 'created_at<' ? now >= n : now <= n)) throw Error('Owner association does not authorize metadata and request authentication');
  }
  if (!schnorr.verify(a[3], digest(`nostr:agent-auth:${agent}:${conditions}`), owner)) throw Error('Invalid owner association signature');
  return a as string[];
}
/** Exact supported generic HTTP relay path, not a host-authenticated socket. */
export function metadataRelay(relay: string): string {
  const u = new URL(relay);
  if (!['ws:', 'wss:'].includes(u.protocol) || u.username || u.password || u.search || u.hash || (u.protocol === 'ws:' && !['127.0.0.1', '[::1]', 'localhost'].includes(u.hostname))) throw Error('Invalid metadata relay');
  u.protocol = u.protocol === 'wss:' ? 'https:' : 'http:';
  return u.href.replace(/\/$/, '');
}
/** Publish only after governing consumer authorization. Query before signing to
 * avoid kind0 same-second tie ordering; exact signed readback is separate from ACK.
 * No retry loop, provider, ACP, or credential backend is opened here.
 */
export async function publishPublicMetadata(input: { relay: string; agent: string; owner: string; secret: string; authTag: string; value: PublicMetadata; check: () => void; retain: (event: Event) => void }): Promise<string> {
  const value = publicMetadata(input.value), base = metadataRelay(input.relay);
  if (publicKey(input.secret) !== input.agent) throw Error('Agent signer mismatch');
  const tags = metadataAuthorization(input.authTag, input.agent, input.owner, Math.floor(Date.now() / 1000));
  async function request(path: string, body: unknown): Promise<unknown> {
    input.check();
    const content = JSON.stringify(body), url = `${base}/${path}`, now = Math.floor(Date.now() / 1000);
    metadataAuthorization(input.authTag, input.agent, input.owner, now);
    const auth = finalizeEvent({ kind: 27235, created_at: now, tags: [['u', url], ['method', 'POST'], ['payload', digest(content).toString('hex')], ['nonce', randomUUID()]], content: '' }, Buffer.from(input.secret, 'hex'));
    const response = await fetch(url, { method: 'POST', redirect: 'error', signal: AbortSignal.timeout(5000), headers: { 'Content-Type': 'application/json', Authorization: `Nostr ${Buffer.from(JSON.stringify(auth)).toString('base64')}`, 'x-auth-tag': input.authTag }, body: content });
    if (!response.ok) { await response.body?.cancel(); throw Error(`Public relay refused ${path} (HTTP ${response.status}); publication unknown`); }
    const reader = response.body?.getReader(); if (!reader) throw Error('Missing public relay response');
    const chunks: Uint8Array[] = []; let size = 0;
    try { for (;;) { const { done, value: part } = await reader.read(); if (done) break; size += part.length; if (size > 65536) throw Error('Public relay response too large'); chunks.push(part); } } finally { await reader.cancel(); }
    input.check(); return JSON.parse(Buffer.concat(chunks).toString());
  }
  async function latest(): Promise<Event | undefined> {
    const rows = await request('query', [{ authors: [input.agent], kinds: [0], limit: 1 }]);
    if (!Array.isArray(rows) || rows.length > 1) throw Error('Invalid public metadata readback');
    if (!rows.length) return undefined;
    const event = rows[0] as Event;
    if (!verifyEvent(event) || event.pubkey !== input.agent || event.kind !== 0) throw Error('Invalid public metadata signature/readback');
    return event;
  }
  const prior = await latest();
  const now = Math.floor(Date.now() / 1000);
  if (prior && prior.created_at >= now) throw Error('Latest kind0 occupies this second or is future-dated; deliberately retry later');
  input.check(); metadataAuthorization(input.authTag, input.agent, input.owner, now);
  const event = finalizeEvent({ kind: 0, created_at: now, tags: [tags], content: JSON.stringify(value) }, Buffer.from(input.secret, 'hex'));
  input.retain(event); // Durable exact candidate before network effect, including unknown outcome.
  const ack = object(await request('events', event));
  if (ack.accepted !== true || ack.event_id !== event.id) throw Error('Public relay did not acknowledge this event; publication unknown');
  const observed = await latest();
  if (observed?.id !== event.id) throw Error('Public event submitted but latest readback differs; publication unknown');
  return `published; signed latest kind0 ${event.id}`;
}
