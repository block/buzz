import { fields, object, parseMessage, type Message } from './protocol.ts';
import { hostPairing, verifyHostRegistration, type HostPairing, type HostRegistration } from './host-registration.ts';
import type { AuthenticatedMessage } from './nostr-codec.ts';

/** Public-key-only retained owner catalog. Registration is not admission or placement. */
export type HostCatalog = { version: 1; owner: string; relay: string; registrations: HostRegistration[] };
/** Session-only, bounded self-signed availability. Never persisted as owner approval. */
export class HostOffers {
  readonly entries = new Map<string, { request: HostPairing; observedAt: number }>();
  receive(catalog: HostCatalog, input: AuthenticatedMessage, now: number): Message {
    const m = parseMessage(input.message);
    if (m.type !== 'availability' || input.sender !== m.host) throw Error('Untrusted availability signer');
    fields(m.body, ['configuration', 'observedAt']);
    const request = hostPairing(m.body.configuration);
    const observedAt = m.body.observedAt;
    if (request.host !== input.sender || request.owner !== catalog.owner || request.relay !== catalog.relay ||
      typeof observedAt !== 'number' || !Number.isSafeInteger(observedAt) || observedAt > now + 1000 || observedAt < now - 6000) throw Error('Invalid availability scope/freshness');
    const prior = this.entries.get(m.host);
    if (!prior && this.entries.size >= 256) throw Error('Availability capacity reached');
    if (!prior || observedAt > prior.observedAt) this.entries.set(m.host, { request, observedAt });
    return m;
  }
  /** Availability grants routing only; command execution still requires agent authority. */
  host(host: string, now: number) {
    const offer = this.entries.get(host);
    return offer && now - offer.observedAt <= 6000 ? offer : undefined;
  }
}
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
export function catalogHost(catalog: HostCatalog, host: string, now = Math.floor(Date.now() / 1000), offers?: HostOffers): { request: HostPairing } {
  const offered = offers?.host(host, now * 1000);
  if (offered && offered.request.owner === catalog.owner && offered.request.relay === catalog.relay) return offered;
  const current = verifyHostCatalog(catalog, catalog.owner, catalog.relay, now);
  const registration = current.registrations.find(r => r.request.host === host);
  if (!registration) throw Error('Host is not registered in owner catalog');
  return registration;
}
/** Reports bind the exact infrastructure signer, never an owner endorsement. */
export function catalogResponse(catalog: HostCatalog, input: AuthenticatedMessage, now = Math.floor(Date.now() / 1000), offers?: HostOffers): Message {
  const m = parseMessage(input.message);
  if (m.type === 'availability' && offers) return offers.receive(catalog, input, now * 1000);
  const registration = catalogHost(catalog, m.host, now, offers);
  if (input.sender !== registration.request.host || !['inventory', 'receipt'].includes(m.type)) throw Error('Untrusted host response');
  if (m.type === 'inventory' && (typeof m.body.observedAt !== 'number' || !Number.isSafeInteger(m.body.observedAt) || m.body.observedAt < 0 || m.body.observedAt > now * 1000 + 1999)) throw Error('Invalid host observation time');
  return m;
}
/** Infrastructure availability and per-agent observations are separate rows. */
export class HostInventory {
  readonly #catalog: HostCatalog;
  readonly #offers = new HostOffers();
  readonly #reports = new Map<string, Message>();
  constructor(catalog: HostCatalog) { this.#catalog = structuredClone(verifyHostCatalog(catalog, catalog.owner, catalog.relay)); }
  receive(input: AuthenticatedMessage, now = Date.now()): Message {
    const m = catalogResponse(this.#catalog, input, Math.floor(now / 1000), this.#offers);
    if (m.type === 'inventory') {
      const observed = m.body.observedAt;
      if (typeof observed !== 'number' || !Number.isSafeInteger(observed) || observed < 0 || observed > now + 1000) throw Error('Invalid host observation time');
      const key = JSON.stringify([m.host, m.agent]);
      const previous = this.#reports.get(key);
      if (!previous && this.#reports.size >= 1000) throw Error('Inventory capacity reached');
      if (!previous || observed > Number(previous.body.observedAt)) this.#reports.set(key, structuredClone(m));
    }
    return m;
  }
  /** Missing/stale reports never mean stopped. Availability is never agent consent. */
  rows(now = Date.now()) {
    const hosts = new Map(this.#catalog.registrations.map(r => [r.request.host, r.request]));
    for (const offer of this.#offers.entries.values()) hosts.set(offer.request.host, offer.request);
    return [...hosts.values()].flatMap(request => {
      const offered = this.#offers.host(request.host, now);
      const legacy = this.#catalog.registrations.find(r => r.request.host === request.host);
      const trusted = !!offered || !!legacy && now < legacy.expires * 1000;
      const reports = [...this.#reports.values()].filter(m => m.host === request.host);
      const infrastructure = {
        host: request.host, label: request.label, agent: undefined as string | undefined,
        status: offered ? 'available infrastructure (no agent authorization)' : legacy && now >= legacy.expires * 1000 ? 'registration-expired' : 'unreachable/unknown',
        report: undefined as Message | undefined,
      };
      const rows = reports.map(report => ({
        host: request.host, label: request.label, agent: report.agent,
        status: !trusted ? legacy ? 'registration-expired' : 'unreachable/unknown' : now - Number(report.body.observedAt) > 6000 ? 'unreachable/unknown' : 'recent-host-report',
        report: trusted ? structuredClone(report) : undefined,
      }));
      return offered || !rows.length ? [infrastructure, ...rows] : rows;
    });
  }
}
