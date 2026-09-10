import { verifyEvent, type Event } from 'nostr-tools/pure';
import { digest, fields, object, text } from './protocol.ts';

/** Direct relay membership for a beehive host against an existing Buzz relay:
 * NIP-11 requirement discovery, host-signed invite claim (NIP-98 HTTP auth),
 * and direct-membership evidence verification against the relay's own signed
 * kind 13534 roster. Interfaces follow the relay contract at block/buzz
 * 051c3a270be9c73da9ab06700bcab7d5552fceaa (api/invites.rs, api/bridge.rs,
 * api/mod.rs, nip11.rs); source verification is not deployed proof, so every
 * check runs against the deployment origin supplied by the caller.
 *
 * Deliberate fences:
 * - The host signer is an injected async boundary; this module never reads a
 *   keychain, never mints invite material, and never accepts or produces
 *   owner NIP-OA/OAuth delegation.
 * - Host registration, agent placement and per-agent authorization are
 *   separate concerns and absent here; the roster is the relay's existing
 *   pubkey/role contract only.
 * - No redirect is ever followed: authorization and invite material must not
 *   cross a redirect or origin change. Every request has a bounded timeout.
 *
 * Proof semantics for the integrator:
 * - `liveAdmission: 'verified'` is a fresh per-request row check: POST /query
 *   runs the relay-membership gate on every request (query_events_authed),
 *   so it is this module's equivalent of the no-OA NIP-42 AUTH live check
 *   without duplicating the WebSocket runtime. It is point-in-time, not
 *   continuous: an already-authenticated WebSocket elsewhere retains its
 *   admission scope until it closes, and revocation is only observed by
 *   re-verification. It says nothing about roles or agent authorization.
 * - `roster.status: 'observed'` is the relay-signed kind 13534 snapshot of
 *   `relay_members` as of its `created_at`. Publication is best-effort after a
 *   committed row change and may lag or fail; `already_member` claims do not
 *   republish it. Old valid rosters are accepted (no freshness gate), and
 *   roster absence never revokes committed or live evidence.
 * - A committed claim response is the relay's transactional readback of the
 *   inserted row, not a publish ACK. None of these establish host
 *   registration, normal enrollment completion or production admission.
 */

/** Bounded HTTP transport boundary. Implementations MUST NOT follow redirects
 * (any 3xx is refused here as well) and must honor the per-request signal. */
export type RelayHttpTransport = (request: {
  url: string;
  method: 'GET' | 'POST';
  headers: Record<string, string>;
  body: string | undefined;
  timeoutMs: number;
  signal: AbortSignal;
}) => Promise<{ status: number; bodyText: string }>;

/** Async host-key boundary. Implementations own key access and must produce a
 * fresh kind 27235 event per request (the relay rejects replayed event ids)
 * covering exactly the given url/method/payload hash. */
export type HostAuthorizationSigner = {
  readonly hostPublicKey: string;
  signHttpAuthentication(request: { url: string; method: 'POST'; payloadSha256Hex: string }): Promise<string>;
};

/** NIP-11 relay membership requirements. `membershipEnforced` is true only
 * when the deployment advertises NIP-43 together with a usable `self` key. */
export type RelayMembershipRequirements = {
  readonly origin: string;
  readonly relaySelf: string | null;
  readonly membershipEnforced: boolean;
  readonly supportedNips: readonly number[];
};

/** Committed invite-claim readback. The operator issued the invite; this
 * module never mints one and never logs or echoes the code. */
export type InviteClaimOutcome = {
  readonly status: 'joined' | 'already_member';
  readonly communityId: string;
  readonly host: string;
  readonly role: string;
  readonly claimedAt: number;
};

/** Relay-signed roster observation for the host key. */
export type RosterObservation = {
  readonly status: 'observed' | 'absent' | 'unavailable' | 'not-checked';
  readonly role?: string;
  readonly createdAt?: number;
  /** Claim committed but the roster does not list the host: publication is
   * best-effort after the commit, so it may lag or have failed. */
  readonly lagging: boolean;
};

/** Truthful observation, never a permanent approved boolean. Precedence:
 * denied > live-verified-member > roster-member > claim-committed > unknown. */
export type DirectMembershipEvidence = {
  readonly relay: string;
  readonly origin: string;
  readonly hostPublicKey: string;
  readonly discovery: 'observed' | 'unavailable';
  readonly relaySelf: string | null;
  readonly membershipEnforced: boolean;
  readonly claim: InviteClaimOutcome | null;
  readonly liveAdmission: 'verified' | 'denied' | 'unavailable' | 'not-checked';
  readonly roster: RosterObservation;
  readonly status: 'live-verified-member' | 'roster-member' | 'claim-committed' | 'denied' | 'unknown';
};

const NIP98_KIND = 27235;
const NIP98_TOLERANCE_SECONDS = 60;
const ROSTER_KIND = 13534;
const MEMBERSHIP_NIP = 43;
const DEFAULT_TIMEOUT_MS = 5000;
const MAX_BODY_BYTES = 262144;

/** Default platform-fetch adapter: redirects are refused, no credentials or
 * proxies are configured, and response bodies are size-bounded. It is the
 * only place this module touches the network. */
export function fetchRelayTransport(fetcher: typeof globalThis.fetch = globalThis.fetch.bind(globalThis)): RelayHttpTransport {
  return async request => {
    const response = await fetcher(request.url, { method: request.method, headers: request.headers, body: request.body, redirect: 'error', signal: request.signal });
    const declared = Number(response.headers.get('content-length'));
    if (declared > MAX_BODY_BYTES) throw Error('Relay response exceeds bounded size');
    const bodyText = await response.text();
    if (bodyText.length > MAX_BODY_BYTES) throw Error('Relay response exceeds bounded size');
    return { status: response.status, bodyText };
  };
}

/** Normal HTTP origin of a relay URL: wss→https, https→https; ws/http only
 * for 127.0.0.1 loopback fixtures, matching the package transport rules. */
export function relayHttpOrigin(relay: string): URL {
  const url = new URL(relay);
  if (url.username || url.password || url.search || url.hash || (url.pathname !== '' && url.pathname !== '/')) throw Error('Relay origin URL expected');
  const secure = url.protocol === 'wss:' || url.protocol === 'https:';
  const loopback = (url.protocol === 'ws:' || url.protocol === 'http:') && url.hostname === '127.0.0.1';
  if (!secure && !loopback) throw Error('Relay requires wss/https (loopback ws/http fixtures only)');
  return new URL(`${secure ? 'https' : 'http'}://${url.host}/`);
}

function boundedTimeout(value: number | undefined): number {
  const timeoutMs = value ?? DEFAULT_TIMEOUT_MS;
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 60000) throw Error('Invalid relay request timeout');
  return timeoutMs;
}

function hexPublicKey(value: unknown, label: string): string {
  if (typeof value !== 'string' || !/^[0-9a-f]{64}$/.test(value)) throw Error(`Invalid ${label}`);
  return value;
}

function requestScope(timeoutMs: number, external?: AbortSignal) {
  const controller = new AbortController();
  const abort = (message: string) => { if (!controller.signal.aborted) controller.abort(Error(message)); };
  const timer = setTimeout(() => abort('Relay HTTP request timeout'), timeoutMs);
  const cancel = () => abort('Relay HTTP request cancelled');
  if (external?.aborted) cancel();
  else external?.addEventListener('abort', cancel, { once: true });
  return { signal: controller.signal, done: () => { clearTimeout(timer); external?.removeEventListener('abort', cancel); } };
}

type RelayRequest = { transport: RelayHttpTransport; url: string; method: 'GET' | 'POST'; headers: Record<string, string>; body: string | undefined; timeoutMs: number; signal?: AbortSignal };

/** One bounded request. The transport race is abandoned (not reported twice)
 * when the scope aborts; redirects are refused before any body is parsed. */
async function relayFetch(request: RelayRequest): Promise<{ status: number; bodyText: string }> {
  const scope = requestScope(request.timeoutMs, request.signal);
  const attempt = request.transport({ url: request.url, method: request.method, headers: request.headers, body: request.body, timeoutMs: request.timeoutMs, signal: scope.signal });
  void attempt.catch(() => {});
  try {
    const response = await Promise.race([attempt, new Promise<never>((_, reject) => {
      // An abort that landed while the transport was being constructed (or a
      // pre-aborted caller signal) has already fired the signal's abort event;
      // a late listener would never fire and the race would hang, so reject
      // immediately instead.
      if (scope.signal.aborted) reject(scope.signal.reason);
      else scope.signal.addEventListener('abort', () => reject(scope.signal.reason), { once: true });
    })]);
    if (response.status >= 300 && response.status < 400) throw Error('Relay redirect refused; authorization and invite material not forwarded');
    if (!Number.isSafeInteger(response.status) || response.status < 200 || response.status > 599 || typeof response.bodyText !== 'string' || response.bodyText.length > MAX_BODY_BYTES) throw Error('Malformed relay transport response');
    return response;
  } finally { scope.done(); }
}

/** Only module-owned, code-free reasons are surfaced; third-party transport
 * error text stays out of messages because it may echo request material. */
function safeReason(error: unknown): string {
  return error instanceof Error && /^(Relay HTTP request timeout|Relay HTTP request cancelled|Relay redirect refused)/.test(error.message) ? error.message : 'transport failure';
}

/** Bounded, control-character-free relay error field only; never a raw body.
 * An excerpt that echoes supplied request material (a hostile relay echoing
 * the invite code back) is coarsened rather than surfaced. */
function relayErrorExcerpt(bodyText: string, secret?: string): string {
  try {
    const value = object(JSON.parse(bodyText));
    if (typeof value.error === 'string' && value.error.length > 0 && value.error.length <= 128 && !/[\x00-\x1f\x7f]/.test(value.error)) {
      return secret !== undefined && value.error.includes(secret) ? 'unspecified relay error' : value.error;
    }
  } catch { /* unparseable relay error bodies stay coarse */ }
  return 'unspecified relay error';
}

/** Strictly typed Nostr event parse; rejects unknown/missing fields. */
function parseEvent(value: unknown): Event {
  const e = object(value);
  fields(e, ['id', 'pubkey', 'created_at', 'kind', 'tags', 'content', 'sig']);
  if (typeof e.id !== 'string' || !/^[0-9a-f]{64}$/.test(e.id) || typeof e.pubkey !== 'string' || !/^[0-9a-f]{64}$/.test(e.pubkey)) throw Error('Invalid event identity');
  if (typeof e.sig !== 'string' || !/^[0-9a-f]{128}$/.test(e.sig)) throw Error('Invalid event signature');
  if (typeof e.content !== 'string' || e.content.length > 4096) throw Error('Invalid event content');
  if (!Number.isSafeInteger(e.created_at) || Number(e.created_at) < 0 || !Number.isSafeInteger(e.kind)) throw Error('Invalid event metadata');
  if (!Array.isArray(e.tags) || e.tags.length > 512) throw Error('Invalid event tags');
  for (const tag of e.tags) {
    if (!Array.isArray(tag) || tag.length < 1 || tag.length > 16) throw Error('Invalid event tag');
    for (const item of tag) if (typeof item !== 'string' || item.length > 1024) throw Error('Invalid event tag value');
  }
  return { id: e.id, pubkey: e.pubkey, created_at: Number(e.created_at), kind: Number(e.kind), tags: e.tags as string[][], content: e.content, sig: e.sig };
}

function tagValues(event: Event, name: string): string[] {
  return event.tags.filter(tag => tag[0] === name && tag.length >= 2).map(tag => tag[1]);
}

/** Signer output is fully re-verified before any request leaves the module,
 * binding the signature to the exact host identity, URL, method and payload. */
async function nip98Authorization(signer: HostAuthorizationSigner, url: string, body: string, now: number): Promise<string> {
  const hostPublicKey = hexPublicKey(signer.hostPublicKey, 'host public key');
  const payloadSha256Hex = digest(body).toString('hex');
  const serialized = await signer.signHttpAuthentication({ url, method: 'POST', payloadSha256Hex });
  let event: Event;
  try { event = parseEvent(JSON.parse(serialized)); } catch { throw Error('Signer returned a malformed NIP-98 event'); }
  if (event.kind !== NIP98_KIND) throw Error('Signer returned a non-NIP-98 event kind');
  if (event.pubkey !== hostPublicKey) throw Error('NIP-98 signer key does not match the specified host identity');
  const [u] = tagValues(event, 'u');
  if (tagValues(event, 'u').length !== 1 || u !== url) throw Error('NIP-98 event is not bound to this relay request URL');
  const [method] = tagValues(event, 'method');
  if (tagValues(event, 'method').length !== 1 || method?.toUpperCase() !== 'POST') throw Error('NIP-98 event does not cover this request method');
  const [payload] = tagValues(event, 'payload');
  if (tagValues(event, 'payload').length !== 1 || payload !== payloadSha256Hex) throw Error('NIP-98 event does not cover the request payload');
  if (Math.abs(event.created_at - now) > NIP98_TOLERANCE_SECONDS) throw Error('NIP-98 event timestamp is outside the relay acceptance window');
  if (!verifyEvent(event)) throw Error('NIP-98 event signature is invalid');
  return `Nostr ${Buffer.from(serialized, 'utf8').toString('base64')}`;
}

function parseClaimResponse(value: unknown): { status: 'joined' | 'already_member'; communityId: string; host: string; role: string } {
  const v = object(value);
  fields(v, ['status', 'community_id', 'host', 'role']);
  if (v.status !== 'joined' && v.status !== 'already_member') throw Error('Invalid claim status');
  return { status: v.status, communityId: text(v.community_id, 128), host: text(v.host, 256), role: text(v.role, 64) };
}

/** Discover relay membership requirements from the deployment's NIP-11
 * document (GET origin with `Accept: application/nostr+json`). NIP-43 is only
 * trustworthy together with a `self` signing key; anything else is reported
 * as not enforced and must never masquerade as verified membership. */
export async function discoverRelayMembershipRequirements(input: { relay: string; transport?: RelayHttpTransport; timeoutMs?: number; signal?: AbortSignal }): Promise<RelayMembershipRequirements> {
  const origin = relayHttpOrigin(input.relay);
  const transport = input.transport ?? fetchRelayTransport();
  const response = await relayFetch({ transport, url: origin.toString(), method: 'GET', headers: { accept: 'application/nostr+json' }, body: undefined, timeoutMs: boundedTimeout(input.timeoutMs), signal: input.signal });
  if (response.status !== 200) throw Error(`Relay information unavailable (HTTP ${response.status})`);
  let document: Record<string, unknown>;
  try { document = object(JSON.parse(response.bodyText)); } catch { throw Error('Malformed relay information document'); }
  const relaySelf = document.self === undefined || document.self === null ? null : hexPublicKey(document.self, 'relay self key');
  const supportedNips: number[] = [];
  if (document.supported_nips !== undefined && document.supported_nips !== null) {
    if (!Array.isArray(document.supported_nips) || document.supported_nips.length > 128) throw Error('Malformed relay supported_nips');
    for (const nip of document.supported_nips) {
      if (!Number.isSafeInteger(nip) || Number(nip) < 0 || Number(nip) > 65535) throw Error('Malformed relay supported_nips');
      supportedNips.push(Number(nip));
    }
  }
  return { origin: origin.toString(), relaySelf, membershipEnforced: relaySelf !== null && supportedNips.includes(MEMBERSHIP_NIP), supportedNips };
}

/** Claim an operator-issued invite with the host's own key (POST
 * /api/invites/claim, NIP-98 with a payload tag; deliberately outside the
 * membership gate). Only the joining host key signs; no owner key is used or
 * accepted. The invite code is request material: it is never logged, echoed,
 * or embedded in any error this module produces. A transport failure after
 * the request left is reported as an unknown commit outcome, never as a
 * clean failure. */
export async function claimRelayMembershipInvite(input: {
  relay: string;
  signer: HostAuthorizationSigner;
  inviteCode: string;
  policyReceipt?: string;
  transport?: RelayHttpTransport;
  timeoutMs?: number;
  signal?: AbortSignal;
  now?: number;
}): Promise<InviteClaimOutcome> {
  const origin = relayHttpOrigin(input.relay);
  hexPublicKey(input.signer.hostPublicKey, 'host public key');
  const inviteCode = text(input.inviteCode, 1024);
  const policyReceipt = input.policyReceipt === undefined ? undefined : text(input.policyReceipt, 2048);
  const now = input.now ?? Math.floor(Date.now() / 1000);
  const transport = input.transport ?? fetchRelayTransport();
  const url = `${origin}api/invites/claim`;
  const body = JSON.stringify(policyReceipt === undefined ? { code: inviteCode } : { code: inviteCode, policy_receipt: policyReceipt });
  const authorization = await nip98Authorization(input.signer, url, body, now);
  let response: { status: number; bodyText: string };
  try {
    response = await relayFetch({ transport, url, method: 'POST', headers: { accept: 'application/json', 'content-type': 'application/json', authorization }, body, timeoutMs: boundedTimeout(input.timeoutMs), signal: input.signal });
  } catch (error) {
    throw Error(`Invite claim request failed before a relay outcome; commit status unknown (${safeReason(error)})`);
  }
  if (response.status !== 200) throw Error(`Invite claim rejected by the relay (HTTP ${response.status}: ${relayErrorExcerpt(response.bodyText, inviteCode)})`);
  let outcome: ReturnType<typeof parseClaimResponse>;
  try { outcome = parseClaimResponse(JSON.parse(response.bodyText)); } catch { throw Error('Malformed invite claim response'); }
  if (outcome.host !== origin.host) throw Error('Invite claim response is bound to a different relay host');
  return { status: outcome.status, communityId: outcome.communityId, host: outcome.host, role: outcome.role, claimedAt: now };
}

/** Verified roster observation for the host key. Only events signed by the
 * NIP-11 `self` key count; unsigned lists and caller labels never do. */
function rosterObservation(events: readonly unknown[], relaySelf: string, hostPublicKey: string, claimCommitted: boolean): RosterObservation {
  let newest: Event | undefined;
  for (const value of events) {
    let event: Event;
    try { event = parseEvent(value); } catch { continue; }
    if (event.kind !== ROSTER_KIND || event.pubkey !== relaySelf || !verifyEvent(event)) continue;
    if (!newest || event.created_at >= newest.created_at) newest = event;
  }
  if (!newest) return { status: 'unavailable', lagging: claimCommitted };
  const roles = new Set<string>();
  try {
    for (const tag of newest.tags) if (tag[0] === 'member' && tag.length >= 3 && tag[1] === hostPublicKey) roles.add(text(tag[2], 64));
  } catch { return { status: 'unavailable', lagging: claimCommitted }; }
  if (roles.size === 0) return { status: 'absent', lagging: claimCommitted };
  if (roles.size > 1) return { status: 'unavailable', lagging: claimCommitted };
  return { status: 'observed', role: [...roles][0], createdAt: newest.created_at, lagging: false };
}

/** Gather direct-membership evidence for a host key against one relay:
 * NIP-11 discovery, then a host-signed membership-gated POST /query read
 * (the live per-request row check) whose response is also the roster source.
 * Signer misuse throws; transport failures downgrade truthfully to
 * 'unavailable'/'unknown' evidence instead of masquerading. */
export async function verifyDirectMembershipEvidence(input: {
  relay: string;
  signer: HostAuthorizationSigner;
  claim?: InviteClaimOutcome;
  transport?: RelayHttpTransport;
  timeoutMs?: number;
  signal?: AbortSignal;
  now?: number;
}): Promise<DirectMembershipEvidence> {
  const origin = relayHttpOrigin(input.relay);
  const hostPublicKey = hexPublicKey(input.signer.hostPublicKey, 'host public key');
  const now = input.now ?? Math.floor(Date.now() / 1000);
  const transport = input.transport ?? fetchRelayTransport();
  const timeoutMs = boundedTimeout(input.timeoutMs);
  const claimInput = input.claim;
  let claim: InviteClaimOutcome | null = null;
  if (claimInput !== undefined) {
    // Re-validate the caller-supplied readback through the wire-shape parser
    // (same field/length rules as a fresh response) before treating it as
    // evidence, and preserve its local commit timestamp.
    if (!Number.isSafeInteger(claimInput.claimedAt) || claimInput.claimedAt < 0) throw Error('Invalid invite claim evidence');
    const parsed = parseClaimResponse({ status: claimInput.status, community_id: claimInput.communityId, host: claimInput.host, role: claimInput.role });
    claim = { status: parsed.status, communityId: parsed.communityId, host: parsed.host, role: parsed.role, claimedAt: claimInput.claimedAt };
  }
  if (claim !== null && claim.host !== origin.host) throw Error('Invite claim evidence is bound to a different relay host');
  let discovery: 'observed' | 'unavailable' = 'unavailable';
  let relaySelf: string | null = null;
  let membershipEnforced = false;
  try {
    const requirements = await discoverRelayMembershipRequirements({ relay: input.relay, transport, timeoutMs, signal: input.signal });
    if (requirements.origin !== origin.toString()) throw Error('Discovery origin mismatch');
    discovery = 'observed';
    relaySelf = requirements.relaySelf;
    membershipEnforced = requirements.membershipEnforced;
  } catch { /* unavailable discovery keeps later evidence unknown, never verified */ }
  let liveAdmission: DirectMembershipEvidence['liveAdmission'] = 'not-checked';
  let roster: RosterObservation = { status: 'not-checked', lagging: false };
  if (relaySelf !== null) {
    const url = `${origin}query`;
    const body = JSON.stringify([{ kinds: [ROSTER_KIND], authors: [relaySelf] }]);
    const authorization = await nip98Authorization(input.signer, url, body, now);
    try {
      const response = await relayFetch({ transport, url, method: 'POST', headers: { accept: 'application/json', 'content-type': 'application/json', authorization }, body, timeoutMs, signal: input.signal });
      if (response.status === 200) {
        const events: unknown = JSON.parse(response.bodyText);
        if (!Array.isArray(events) || events.length > 512) throw Error('Malformed relay query response');
        roster = rosterObservation(events, relaySelf, hostPublicKey, claim !== null);
        // Only an enforced relay's gate makes a 200 a live row check; an open
        // relay serves the same read to anyone and must not masquerade.
        if (membershipEnforced) liveAdmission = 'verified';
      } else if (membershipEnforced && response.status === 403 && relayErrorExcerpt(response.bodyText) === 'relay_membership_required') {
        liveAdmission = 'denied';
        roster = { status: 'unavailable', lagging: claim !== null };
      } else {
        liveAdmission = 'unavailable';
        roster = { status: 'unavailable', lagging: claim !== null };
      }
    } catch { liveAdmission = 'unavailable'; roster = { status: 'unavailable', lagging: claim !== null }; }
  }
  const status: DirectMembershipEvidence['status'] = liveAdmission === 'denied' ? 'denied'
    : liveAdmission === 'verified' ? 'live-verified-member'
    : roster.status === 'observed' ? 'roster-member'
    : claim !== null ? 'claim-committed'
    : 'unknown';
  return { relay: input.relay, origin: origin.toString(), hostPublicKey, discovery, relaySelf, membershipEnforced, claim, liveAdmission, roster, status };
}
