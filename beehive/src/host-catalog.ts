import { fields, object, parseMessage, type Message } from './protocol.ts';
import { hostPairing, verifyHostRegistration, type HostRegistration } from './host-registration.ts';
import type { AuthenticatedMessage } from './nostr-codec.ts';

/** Public-key-only retained owner catalog. Registration is not admission or placement. */
export type HostCatalog = { version: 1; owner: string; relay: string; registrations: HostRegistration[] };
/** Reverify persisted input, including signatures, scope and expiry, on every use. */
export function verifyHostCatalog(value: unknown, owner: string, relay: string, now = Math.floor(Date.now() / 1000)): HostCatalog {
  const c = object(value);
  fields(c, ['version', 'owner', 'relay', 'registrations']);
  if (c.version !== 1 || c.owner !== owner || c.relay !== relay || !Array.isArray(c.registrations) || c.registrations.length > 256) throw Error('Invalid owner host catalog scope');
  const keys = new Set<string>();
  const registrations = c.registrations.map(value => {
    const request = hostPairing(object(value).request);
    if (request.owner !== owner || request.relay !== relay) throw Error('Host catalog registration scope mismatch');
    const registration = verifyHostRegistration(value, request, now);
    if (keys.has(request.host)) throw Error('Duplicate host catalog identity');
    keys.add(request.host);
    return registration;
  });
  return { version: 1, owner, relay, registrations };
}
/** Host PUBLIC KEY is the route. Duplicate display labels never confer authority. */
export function catalogHost(catalog: HostCatalog, host: string, now = Math.floor(Date.now() / 1000)): HostRegistration {
  const current = verifyHostCatalog(catalog, catalog.owner, catalog.relay, now);
  const registration = current.registrations.find(r => r.request.host === host);
  if (!registration) throw Error('Host is not registered in owner catalog');
  return registration;
}
/** Only the exact registered infrastructure signer may supply this host's reports. */
export function catalogResponse(catalog: HostCatalog, input: AuthenticatedMessage, now = Math.floor(Date.now() / 1000)): Message {
  const m = parseMessage(input.message);
  const registration = catalogHost(catalog, m.host, now);
  if (input.sender !== registration.request.host || !['inventory', 'receipt'].includes(m.type)) throw Error('Untrusted host response');
  if (m.type === 'inventory' && (typeof m.body.observedAt !== 'number' || !Number.isSafeInteger(m.body.observedAt) || m.body.observedAt < 0 || m.body.observedAt > now * 1000 + 1999)) throw Error('Invalid host observation time');
  return m;
}
/** Existing inventory consumer with registration and freshness fences, not a second client. */
export class HostInventory {
  readonly #catalog: HostCatalog;
  readonly #reports = new Map<string, Message>();
  constructor(catalog: HostCatalog) { this.#catalog = structuredClone(verifyHostCatalog(catalog, catalog.owner, catalog.relay)); }
  receive(input: AuthenticatedMessage, now = Date.now()): Message {
    const m = catalogResponse(this.#catalog, input, Math.floor(now / 1000));
    if (m.type === 'inventory') {
      const observed = m.body.observedAt;
      if (typeof observed !== 'number' || !Number.isSafeInteger(observed) || observed < 0 || observed > now + 1000) throw Error('Invalid host observation time');
      const key = JSON.stringify([m.host, m.agent]);
      const previous = this.#reports.get(key);
      if (!previous || observed > Number(previous.body.observedAt)) this.#reports.set(key, structuredClone(m));
    }
    return m;
  }
  /** Missing/stale reports never mean stopped. Expired registration never means trusted. */
  rows(now = Date.now()) {
    return this.#catalog.registrations.flatMap(r => {
      const reports = [...this.#reports.values()].filter(m => m.host === r.request.host);
      const trusted = now < r.expires * 1000;
      return (reports.length ? reports : [undefined]).map(report => ({
        host: r.request.host, label: r.request.label, agent: report?.agent,
        status: !trusted ? 'registration-expired' : !report || now - Number(report.body.observedAt) > 6000 ? 'unreachable/unknown' : 'recent-host-report',
        report: trusted && report ? structuredClone(report) : undefined,
      }));
    });
  }
}
