import { finalizeEvent, generateSecretKey, getPublicKey, getEventHash, verifyEvent, type Event } from 'nostr-tools/pure';
import { v2 } from 'nostr-tools/nip44';
import { fields, object, parseMessage, serializeManagement, MANAGEMENT_WIRE_BYTES, type Message } from './protocol.ts';

const domain = 'beehive-management-v1';
/** Authenticated inner message; never infer the sender from the ephemeral outer key. */
export type AuthenticatedMessage = { sender: string; message: Message };
/** Standard NIP-59 seal + wrap, using current outer time to meet Buzz's ±900s admission.
 * NIP-59 timestamp randomization is deliberately omitted (privacy tradeoff). Stable
 * operation identity remains Message.id, NOT the rumor, seal, or wrap event id.
 */
export function wrapManagement(message: Message, secret: Uint8Array, recipient: string, now = Math.floor(Date.now() / 1000)): Event {
  const content = serializeManagement(message);
  const rumor = { kind: 14, pubkey: getPublicKey(secret), created_at: now, tags: [['p', recipient], ['subject', domain]], content };
  const seal = finalizeEvent({ kind: 13, created_at: now, tags: [], content: v2.encrypt(JSON.stringify({ ...rumor, id: getEventHash(rumor) }), v2.utils.getConversationKey(secret, recipient)) }, secret);
  const ephemeral = generateSecretKey();
  return finalizeEvent({ kind: 1059, created_at: now, tags: [['p', recipient]], content: v2.encrypt(JSON.stringify(seal), v2.utils.getConversationKey(ephemeral, recipient)) }, ephemeral);
}
function verified(value: unknown, kind: number): Event {
  const e = object(value);
  fields(e, ['id', 'pubkey', 'created_at', 'kind', 'tags', 'content', 'sig']);
  if (e.kind !== kind || typeof e.content !== 'string' || Buffer.byteLength(e.content) > 262144 || !verifyEvent(e as Event)) throw Error('Invalid signed event');
  return e as Event;
}
/** Verify both signatures, recipient, rumor hash and seal/rumor author binding before returning plaintext. */
export function unwrapManagement(value: unknown, secret: Uint8Array): AuthenticatedMessage {
  const wrap = verified(value, 1059);
  const recipient = getPublicKey(secret);
  if (JSON.stringify(wrap.tags) !== JSON.stringify([['p', recipient]])) throw Error('Wrong recipient');
  const seal = verified(JSON.parse(v2.decrypt(wrap.content, v2.utils.getConversationKey(secret, wrap.pubkey))), 13);
  if (seal.tags.length) throw Error('Unexpected seal tags');
  const rumor = object(JSON.parse(v2.decrypt(seal.content, v2.utils.getConversationKey(secret, seal.pubkey))));
  fields(rumor, ['id', 'pubkey', 'created_at', 'kind', 'tags', 'content']);
  if (rumor.pubkey !== seal.pubkey || rumor.kind !== 14 || typeof rumor.content !== 'string' || Buffer.byteLength(rumor.content) > MANAGEMENT_WIRE_BYTES || JSON.stringify(rumor.tags) !== JSON.stringify([['p', recipient], ['subject', domain]]) || rumor.id !== getEventHash(rumor as unknown as Event)) throw Error('Invalid management rumor');
  return { sender: seal.pubkey, message: parseMessage(JSON.parse(rumor.content)) };
}
/** Tracer authority boundary. Host-to-host Move traffic is intentionally denied until
 * source authority + owner-attested host catalog is integrated with retained journals.
 */
export function authorizeManagement(input: AuthenticatedMessage, owner: string, hosts: Readonly<Record<string, string>>, role: { host: string } | 'owner'): Message {
  const { sender, message } = input;
  if (role === 'owner') {
    if (!['inventory', 'receipt'].includes(message.type) || hosts[message.host] !== sender) throw Error('Untrusted host response');
  } else {
    if (message.host !== role.host || sender !== owner || !['metadata', 'profile', 'inspect', 'save', 'start', 'restart', 'stop', 'move'].includes(message.type)) throw Error('Unauthorized owner command');
  }
  return message;
}
