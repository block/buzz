import { stripVTControlCharacters } from 'node:util';
import { WebSocket } from 'ws';
import { verifyEvent, type Event } from 'nostr-tools/pure';
import { randomUUID } from 'node:crypto';
import type { RegisteredAgent } from './settings.ts';

/** Signed public kind0 only; profile metadata is not execution authority. */
export function profileFromEvents(events: Event[], author: string, relay: string, now = Math.floor(Date.now() / 1000)): Pick<RegisteredAgent, 'profile' | 'profileState'> {
  const valid = events.filter(e => e.kind === 0 && e.pubkey === author && e.created_at <= now + 60 && e.content.length <= 16384 && verifyEvent({ id: e.id, pubkey: e.pubkey, kind: e.kind, created_at: e.created_at, tags: e.tags, content: e.content, sig: e.sig })).sort((a,b) => b.created_at - a.created_at || b.id.localeCompare(a.id));
  for (const event of valid) {
    try {
      const value = JSON.parse(event.content); if (!value || typeof value !== 'object' || Array.isArray(value)) continue;
      const clean = (v: unknown, max: number) => typeof v === 'string' ? stripVTControlCharacters(v).replace(/[\x00-\x1f\x7f-\x9f]/g, '').slice(0,max) : undefined;
      const name = clean(value.display_name,128) || clean(value.name,128), about = clean(value.about,2048), picture = clean(value.picture,2048);
      return { profileState: 'found', profile: { relay, ...(name ? { name } : {}), ...(about ? { about } : {}), ...(picture && /^https:\/\//.test(picture) ? { picture } : {}) } };
    } catch { /* Malformed public metadata is not a valid profile. */ }
  }
  return { profileState: 'none' };
}
/** Bounded read-only unauthenticated query. An AUTH gate is unavailable, not empty.
 * Settlement waits for socket close; no private key is given to this consumer. */
export function fetchAgentProfile(relay: string, author: string, signal: AbortSignal): Promise<Pick<RegisteredAgent, 'profile' | 'profileState'>> {
  signal.throwIfAborted();
  const url = new URL(relay);
  if (url.username || url.password || url.hash || !(url.protocol === 'wss:' || url.protocol === 'ws:' && url.hostname === '127.0.0.1') || !/^[0-9a-f]{64}$/.test(author)) throw Error('Invalid profile relay or public key');
  return new Promise(resolve => {
    const socket = new WebSocket(relay, { handshakeTimeout: 2000, maxPayload: 32768 });
    const id = randomUUID(), events: Event[] = [];
    let result: Pick<RegisteredAgent, 'profile' | 'profileState'> = { profileState: 'unavailable' }, ending = false;
    const finish = (value = result) => { if (ending) return; ending = true; result = value; socket.terminate(); };
    const abort = () => finish();
    const timer = setTimeout(finish, 4000);
    signal.addEventListener('abort', abort, { once: true });
    socket.on('open', () => { if (!ending) socket.send(JSON.stringify(['REQ',id,{ kinds: [0], authors: [author], limit: 32 }])); });
    socket.on('error', () => finish());
    socket.on('message', bytes => {
      if (ending) return;
      try { const frame = JSON.parse(bytes.toString());
        if (frame[0] === 'EVENT' && frame[1] === id) { if (events.length >= 32) return finish(); events.push(frame[2]); }
        else if (frame[0] === 'EOSE' && frame[1] === id) finish(profileFromEvents(events,author,relay));
        else if (['AUTH','CLOSED'].includes(frame[0])) finish();
      } catch { finish(); }
    });
    socket.on('close', () => { clearTimeout(timer); signal.removeEventListener('abort',abort); resolve(result); });
    if (signal.aborted) abort();
  });
}
