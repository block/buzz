import { existsSync, mkdirSync, readdirSync, statSync, lstatSync } from 'node:fs';
import { join } from 'node:path';
import { connect, validateRelayURL } from './client.ts';
import { digest, fields, object, open, publicKey, seal, type Envelope, type Message } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';

type Intent = { envelope: Envelope; request: Message; receipt?: Message; published: boolean; blocked?: boolean };
const fingerprint = (m: Message) => digest(JSON.stringify(m)).toString('hex');
function matches(request: Message, receipt: Message) {
  return receipt.type === 'receipt' && receipt.host === request.host && receipt.agent === request.agent
    && receipt.body.operation === request.id && receipt.body.fingerprint === fingerprint(request)
    && typeof receipt.body.result === 'string' && receipt.body.result.length > 0;
}
/** Local operator journal, scoped to exact relay URL and signer; no agent keys.
 * Immutable intent files precede all network effects. Receipt files are separate
 * so another reader's old publication observation cannot erase a terminal result.
 */
export function managementClient(root: string, url: string, secret: string, receive: (m: Message) => void, changed: () => void = () => {}) {
  validateRelayURL(url);
  const scope = digest(JSON.stringify([url, publicKey(secret)])).toString('hex');
  const dir = join(root, scope);
  mkdirSync(dir, { recursive: true, mode: 0o700 });
  for (const path of [root, dir]) {
    const stat = lstatSync(path);
    if (!stat.isDirectory() || (stat.mode & 0o077)) throw Error('Management journal directory must be owner-only');
  }
  const intents = new Map<string, Intent>();
  const names = readdirSync(dir);
  if (names.length > 4000 || names.filter(n => n.endsWith('.intent')).length > 1000) throw Error('Management journal full');
  for (const name of names.filter(n => n.endsWith('.intent'))) {
    const path = join(dir, name);
    if (statSync(path).size > 70000) throw Error('Oversized intent');
    const value = object(readPrivate(path)); fields(value, ['scope', 'envelope']);
    if (value.scope !== scope) throw Error('Wrong journal scope');
    const request = open(value.envelope, secret);
    if (!['save','start','stop'].includes(request.type) || name !== `${request.id}.intent`) throw Error('Invalid intent');
    const intent: Intent = { envelope: value.envelope as Envelope, request, published: false };
    const receiptPath = join(dir, `${request.id}.receipt`);
    if (existsSync(receiptPath)) {
      if (statSync(receiptPath).size > 70000) throw Error('Oversized receipt');
      const receipt = open(readPrivate(receiptPath), secret);
      if (!matches(request, receipt)) throw Error('Unmatched journal receipt');
      intent.receipt = receipt;
    }
    const blockedPath = join(dir, `${request.id}.blocked`);
    if (existsSync(blockedPath)) {
      if (statSync(blockedPath).size > 70000) throw Error('Oversized blocked intent');
      const blocked = open(readPrivate(blockedPath), secret);
      if (fingerprint(blocked) !== fingerprint(request)) throw Error('Invalid blocked intent');
      intent.blocked = true;
    }
    intents.set(request.id, intent);
  }
  let closed = false;
  function replay() {
    for (const intent of intents.values()) if (!intent.receipt && !intent.blocked) {
      try { transport.sendEnvelope(intent.envelope); } catch { break; } // Intent remains durable.
    }
    changed();
  }
  const transport = connect(url, secret, m => {
    if (m.type === 'receipt') {
      const intent = intents.get(String(m.body.operation));
      if (intent && !intent.receipt && matches(intent.request, m)) {
        // Persist before exposing completion. Failure propagates, never reports success.
        const path = join(dir, `${intent.request.id}.receipt`);
        try { writePrivate(path, seal(m, secret), true); } catch (error) {
          if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error;
        }
        const persisted = open(readPrivate(path), secret);
        if (!matches(intent.request, persisted)) throw Error('Unmatched durable receipt');
        intent.receipt = persisted; changed();
      }
    } else {
      const intent = intents.get(m.id);
      if (intent && fingerprint(intent.request) === fingerprint(m)) { intent.published = true; changed(); }
    }
    receive(m);
  }, replay, code => {
    if (code === 1008) for (const intent of intents.values()) if (!intent.receipt) {
      // Relay policy errors do not identify a committed host result. Preserve
      // UNKNOWN, but never blindly republish rejected traffic across reopen.
      writePrivate(join(dir, `${intent.request.id}.blocked`), intent.envelope);
      intent.blocked = true;
    }
    changed();
  });
  const ready = transport.ready.then(replay);
  return {
    ready,
    get connected() { return transport.socket.readyState === 1; },
    get socket() { return transport.socket; },
    submit(request: Message) {
      if (closed) throw Error('UI closed');
      if (!['save','start','stop'].includes(request.type)) throw Error('Invalid operation');
      if (intents.size >= 1000) throw Error('Management journal full; no operation submitted');
      if (intents.has(request.id) || existsSync(join(dir, `${request.id}.intent`))) throw Error('Operation ID already prepared');
      const envelope = seal(request, secret);
      const immutable = open(envelope, secret);
      writePrivate(join(dir, `${request.id}.intent`), { scope, envelope }, true);
      intents.set(request.id, { request: immutable, envelope, published: false });
      replay();
    },
    status() {
      return [...intents.values()].map(i => ({ request: structuredClone(i.request),
        state: i.receipt ? (['accepted','saved; running configuration unchanged'].includes(String(i.receipt.body.result)) ? 'completed' : i.receipt.body.result === 'interrupted-reconcile-locally' ? 'unknown' : 'failed') : i.blocked ? 'unknown' : 'pending',
        publication: i.blocked ? 'relay policy failure; automatic retry disabled' : i.receipt ? 'host terminal receipt' : i.published ? 'relay observed; not host admission' : 'unconfirmed',
        result: i.receipt?.body.result }));
    },
    close() { closed = true; transport.close(); },
  };
}
