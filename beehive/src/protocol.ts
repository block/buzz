import { createHash, createCipheriv, createDecipheriv, randomBytes, randomUUID } from 'node:crypto';
import { schnorr } from '@noble/curves/secp256k1';

/** Experimental private wire protocol; deliberately not a Nostr kind allocation.
 * Host-level availability has agent='' and revision=0, never an agent inventory row. */
export type Message = { v: 1; id: string; host: string; agent: string; type: 'metadata' | 'availability' | 'profile' | 'inspect' | 'save' | 'start' | 'restart' | 'stop' | 'move' | 'prepare' | 'prepared' | 'grant' | 'inventory' | 'receipt'; revision: number; body: Record<string, unknown> };
export type Envelope = { v: 1; owner: string; nonce: string; ciphertext: string; tag: string; signature: string };
/** Reject unknown envelope/message fields, oversized values and ambiguous revisions. */
export function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw Error('Expected object');
  return value as Record<string, unknown>;
}
export function text(value: unknown, max = 256): string {
  if (typeof value !== 'string' || !value.length || value.length > max || /[\x00-\x1f\x7f]/.test(value)) throw Error('Invalid text');
  return value;
}
export function fields(value: Record<string, unknown>, names: string[]) {
  if (Object.keys(value).sort().join() !== names.sort().join()) throw Error('Unexpected fields');
}
export function parseMessage(value: unknown): Message {
  const m = object(value);
  fields(m, ['v','id','host','agent','type','revision','body']);
  if (m.v !== 1 || !['metadata','availability','profile','inspect','save','start','restart','stop','move','prepare','prepared','grant','inventory','receipt'].includes(String(m.type)) || !Number.isSafeInteger(m.revision) || Number(m.revision) < 0) throw Error('Invalid message');
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(text(m.id))) throw Error('Invalid operation ID');
  text(m.host);
  if (m.type === 'availability') { if (m.agent !== '' || m.revision !== 0) throw Error('Availability is not an agent report'); }
  else text(m.agent);
  object(m.body);
  return m as Message;
}
export function digest(value: string): Buffer { return createHash('sha256').update(value).digest(); }
export function publicKey(secret: string): string { return Buffer.from(schnorr.getPublicKey(secret)).toString('hex'); }
export function newKey(): string { return Buffer.from(schnorr.utils.randomPrivateKey()).toString('hex'); }
function bytes(e: Omit<Envelope, 'signature'>): string { return JSON.stringify([e.v,e.owner,e.nonce,e.ciphertext,e.tag]); }
function encryptionKey(secret: string): Buffer { return digest(`beehive-private-v1:${secret}`); }
/** Encrypt before relay publication; the relay only receives the owner's public key. */
export function seal(message: Message, secret: string): Envelope {
  parseMessage(message);
  if (Buffer.byteLength(JSON.stringify(message)) > 32768) throw Error('Management message exceeds bounded wire size');
  const nonce = randomBytes(12);
  const cipher = createCipheriv('aes-256-gcm', encryptionKey(secret), nonce);
  cipher.setAAD(Buffer.from('beehive-private-v1'));
  const ciphertext = Buffer.concat([cipher.update(JSON.stringify(message)), cipher.final()]).toString('hex');
  const e = { v: 1 as const, owner: publicKey(secret), nonce: nonce.toString('hex'), ciphertext, tag: cipher.getAuthTag().toString('hex') };
  return { ...e, signature: Buffer.from(schnorr.sign(digest(bytes(e)), secret)).toString('hex') };
}
export function verifyEnvelope(value: unknown, owner: string): Envelope {
  const e = object(value);
  fields(e, ['v','owner','nonce','ciphertext','tag','signature']);
  if (e.v !== 1 || e.owner !== owner) throw Error('Wrong owner/protocol');
  for (const [key, length] of [['nonce',24],['tag',32],['signature',128]] as const) {
    if (typeof e[key] !== 'string' || e[key].length !== length || !/^[0-9a-f]+$/.test(e[key])) throw Error('Invalid envelope');
  }
  if (typeof e.ciphertext !== 'string' || e.ciphertext.length > 65536 || !/^(?:[0-9a-f]{2})+$/.test(e.ciphertext)) throw Error('Invalid ciphertext');
  const envelope = e as Envelope;
  if (!schnorr.verify(envelope.signature, digest(bytes(envelope)), owner)) throw Error('Invalid signature');
  return envelope;
}
export function open(value: unknown, secret: string): Message {
  const e = verifyEnvelope(value, publicKey(secret));
  const decipher = createDecipheriv('aes-256-gcm', encryptionKey(secret), Buffer.from(e.nonce,'hex'));
  decipher.setAAD(Buffer.from('beehive-private-v1'));
  decipher.setAuthTag(Buffer.from(e.tag,'hex'));
  return parseMessage(JSON.parse(Buffer.concat([decipher.update(Buffer.from(e.ciphertext,'hex')),decipher.final()]).toString()));
}
export function message(type: Message['type'], host: string, agent: string, revision = 0, body: Record<string, unknown> = {}): Message {
  return { v: 1, id: randomUUID(), type, host, agent, revision, body };
}
