import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { generateSecretKey, getPublicKey, finalizeEvent, verifyEvent, type Event } from 'nostr-tools/pure';
import {
  claimRelayMembershipInvite,
  discoverRelayMembershipRequirements,
  fetchRelayTransport,
  verifyDirectMembershipEvidence,
  type HostAuthorizationSigner,
  type RelayHttpTransport,
} from '../src/direct-membership.ts';

const NOW = 1_700_000_000;
const COMMUNITY_ID = '7e4e4d21-8d64-4c9a-9d3f-1b0f3d5a9c2e';

/** Narrow executable model of the relay's HTTP membership contract at block/buzz
 * 051c3a270be9c73da9ab06700bcab7d5552fceaa (api/invites.rs, api/bridge.rs,
 * api/mod.rs, store/relay_members.rs): NIP-11 discovery, NIP-98-signed POST
 * /api/invites/claim outside the membership gate, membership-gated POST
 * /query, and the kind 13534 roster ('-' marker plus ['member', pubkey, role]
 * tags) signed by the relay self key. NOT the deployed Rust relay: no tenant
 * DB, no HMAC invite oracle, no replay cache — fixtures grant fixture-policy
 * member rows only. The module under test is the client. */

function rosterEvent(relayKey: Uint8Array, members: readonly (readonly [string, string])[], createdAt: number): Event {
  return finalizeEvent({ kind: 13534, created_at: createdAt, tags: [['-'], ...members.map(([pubkey, role]) => ['member', pubkey, role])], content: '' }, relayKey);
}

function hostSigner(secretKey: Uint8Array, options?: {
  key?: Uint8Array;   // sign with a key other than the declared host identity
  now?: number;       // created_at override
  url?: string;       // misbound u tag
  payload?: string;   // misbound payload hash
  kind?: number;      // wrong event kind
  corrupt?: boolean;  // well-formed event, invalid signature
  raw?: () => string;  // arbitrary signer output
}): HostAuthorizationSigner {
  return {
    hostPublicKey: getPublicKey(secretKey),
    async signHttpAuthentication(request) {
      if (options?.raw) return options.raw();
      const event = finalizeEvent({
        kind: options?.kind ?? 27235,
        created_at: options?.now ?? NOW,
        tags: [['u', options?.url ?? request.url], ['method', request.method], ['payload', options?.payload ?? request.payloadSha256Hex]],
        content: '',
      }, options?.key ?? secretKey);
      return JSON.stringify(options?.corrupt ? { ...event, sig: `${event.sig.slice(0, -1)}${event.sig.endsWith('0') ? '1' : '0'}` } : event);
    },
  };
}

/** Server-side NIP-98 verification — the relay boundary the module must
 * satisfy: exact full URL, method, payload SHA-256 of the exact body, ±60s,
 * and a valid signature by the declaring key. */
function verifyNip98ServerSide(authorization: string | undefined, url: string, body: string): string {
  assert.ok(typeof authorization === 'string' && authorization.startsWith('Nostr '), 'Nostr authorization header');
  const event = JSON.parse(Buffer.from(authorization.slice('Nostr '.length), 'base64').toString('utf8')) as Event;
  assert.ok(verifyEvent(event), 'NIP-98 signature');
  assert.equal(event.kind, 27235);
  assert.ok(Math.abs(event.created_at - NOW) <= 60, 'NIP-98 time window');
  const values = (name: string) => event.tags.filter(tag => tag[0] === name).map(tag => tag[1]);
  assert.deepEqual(values('u'), [url], 'exact full URL binding');
  assert.deepEqual(values('method'), ['POST'], 'method binding');
  assert.deepEqual(values('payload'), [createHash('sha256').update(body).digest('hex')], 'payload hash of the exact body');
  return event.pubkey;
}

type ScriptedRelay = {
  readonly transport: RelayHttpTransport;
  readonly requests: { url: string; method: string; headers: Record<string, string>; body?: string }[];
  readonly members: Map<string, string>;
};

/** Scripted in-memory relay (contract model above) with per-request capture. */
function scriptedRelay(input: {
  relayKey: Uint8Array;
  enforced: boolean;
  self?: string | null;
  members?: readonly (readonly [string, string])[];
  roster?: readonly unknown[];
  communityId?: string;
  host?: string;
  claimOutcome?: (code: string, authorization: string | undefined) => { status: number; bodyText: string };
  queryOutcome?: (pubkey: string, authorization: string | undefined) => { status: number; bodyText: string };
}): ScriptedRelay {
  const host = input.host ?? 'relay.example.com';
  const self = input.self === undefined ? getPublicKey(input.relayKey) : input.self;
  const members = new Map<string, string>(input.members ?? []);
  const requests: ScriptedRelay['requests'] = [];
  const transport: RelayHttpTransport = async request => {
    requests.push({ url: request.url, method: request.method, headers: { ...request.headers }, body: request.body });
    const url = new URL(request.url);
    if (request.method === 'GET' && url.pathname === '/') {
      const document: Record<string, unknown> = { name: 'fixture', supported_nips: input.enforced ? [1, 43] : [1] };
      if (self) document.self = self;
      return { status: 200, bodyText: JSON.stringify(document) };
    }
    if (request.method === 'POST' && url.pathname === '/api/invites/claim') {
      let code = '';
      try { code = String(JSON.parse(request.body ?? '{}').code); } catch { /* overrides still run */ }
      if (input.claimOutcome) return input.claimOutcome(code, request.headers.authorization);
      let pubkey: string;
      try { pubkey = verifyNip98ServerSide(request.headers.authorization, request.url, request.body ?? ''); }
      catch { return { status: 401, bodyText: JSON.stringify({ error: 'unauthorized' }) }; }
      const wasMember = members.has(pubkey);
      members.set(pubkey, 'member');
      return { status: 200, bodyText: JSON.stringify({ status: wasMember ? 'already_member' : 'joined', community_id: input.communityId ?? COMMUNITY_ID, host, role: 'member' }) };
    }
    if (request.method === 'POST' && url.pathname === '/query') {
      let pubkey: string;
      try { pubkey = verifyNip98ServerSide(request.headers.authorization, request.url, request.body ?? ''); }
      catch { return { status: 401, bodyText: JSON.stringify({ error: 'unauthorized' }) }; }
      if (input.queryOutcome) return input.queryOutcome(pubkey, request.headers.authorization);
      if (input.enforced && !members.has(pubkey)) return { status: 403, bodyText: JSON.stringify({ error: 'relay_membership_required' }) };
      return { status: 200, bodyText: JSON.stringify(input.roster ?? []) };
    }
    return { status: 404, bodyText: JSON.stringify({ error: 'not_found' }) };
  };
  return { transport, requests, members };
}

test('positive wire flow: discovery, exact NIP-98 claim/query binding, verified roster readback, old roster accepted', async () => {
  const relayKey = generateSecretKey(), hostKey = generateSecretKey();
  const hostPubkey = getPublicKey(hostKey);
  const relaySelf = getPublicKey(relayKey);
  const fixture = scriptedRelay({ relayKey, enforced: true, roster: [rosterEvent(relayKey, [[hostPubkey, 'member']], NOW - 3600)] });
  const input = { relay: 'wss://relay.example.com', signer: hostSigner(hostKey), inviteCode: 'v2.fixture-code', transport: fixture.transport, now: NOW };

  const claim = await claimRelayMembershipInvite(input);
  assert.deepEqual(claim, { status: 'joined', communityId: COMMUNITY_ID, host: 'relay.example.com', role: 'member', claimedAt: NOW });
  const secondClaim = await claimRelayMembershipInvite(input);
  assert.equal(secondClaim.status, 'already_member'); // idempotent committed-row readback
  const claimRequest = fixture.requests.at(-1)!;
  assert.equal(claimRequest.method, 'POST');
  assert.equal(claimRequest.url, 'https://relay.example.com/api/invites/claim');
  assert.deepEqual(JSON.parse(claimRequest.body!), { code: 'v2.fixture-code' });
  // Server-side NIP-98 verification inside the fixture proves the exact URL,
  // method and body-hash binding; a misbinding would have been a 401 above.

  const requirements = await discoverRelayMembershipRequirements({ relay: 'wss://relay.example.com', transport: fixture.transport });
  assert.deepEqual(requirements, { origin: 'https://relay.example.com/', relaySelf, membershipEnforced: true, supportedNips: [1, 43] });
  const discoveryRequest = fixture.requests.at(-1)!;
  assert.equal(discoveryRequest.method, 'GET');
  assert.equal(discoveryRequest.url, 'https://relay.example.com/');
  assert.equal(discoveryRequest.headers.accept, 'application/nostr+json');

  const evidence = await verifyDirectMembershipEvidence({ relay: 'wss://relay.example.com', signer: hostSigner(hostKey), claim, transport: fixture.transport, now: NOW });
  assert.equal(evidence.status, 'live-verified-member');
  assert.equal(evidence.liveAdmission, 'verified');
  assert.deepEqual(evidence.roster, { status: 'observed', role: 'member', createdAt: NOW - 3600, lagging: false });
  assert.deepEqual(evidence.claim, claim);
  const queryRequest = fixture.requests.at(-1)!;
  assert.equal(queryRequest.method, 'POST');
  assert.equal(queryRequest.url, 'https://relay.example.com/query');
  assert.deepEqual(JSON.parse(queryRequest.body!), [{ kinds: [13534], authors: [relaySelf] }]);

  // An old (30-day) valid roster is accepted: no created_at freshness gate.
  const old = scriptedRelay({ relayKey, enforced: true, members: [[hostPubkey, 'member']], roster: [rosterEvent(relayKey, [[hostPubkey, 'member']], NOW - 30 * 86400)] });
  const oldEvidence = await verifyDirectMembershipEvidence({ relay: 'wss://relay.example.com', signer: hostSigner(hostKey), transport: old.transport, now: NOW });
  assert.equal(oldEvidence.status, 'live-verified-member');
  assert.deepEqual(oldEvidence.roster, { status: 'observed', role: 'member', createdAt: NOW - 30 * 86400, lagging: false });

  // Lagging publication: the member row exists (gate passes) but the roster
  // snapshot does not list the host yet — reported honestly, never as failure.
  const lagging = scriptedRelay({ relayKey, enforced: true, members: [[hostPubkey, 'member']], roster: [rosterEvent(relayKey, [], NOW)] });
  const laggingEvidence = await verifyDirectMembershipEvidence({ relay: 'wss://relay.example.com', signer: hostSigner(hostKey), claim, transport: lagging.transport, now: NOW });
  assert.equal(laggingEvidence.status, 'live-verified-member');
  assert.deepEqual(laggingEvidence.roster, { status: 'absent', lagging: true });
});

test('misbound or forged signer output and forged rosters never verify or reach the wire', async () => {
  const relayKey = generateSecretKey(), hostKey = generateSecretKey(), otherKey = generateSecretKey();
  const fixture = scriptedRelay({ relayKey, enforced: true });
  const input = { relay: 'wss://relay.example.com', inviteCode: 'v2.fixture-code', transport: fixture.transport, now: NOW };
  await assert.rejects(claimRelayMembershipInvite({ ...input, signer: hostSigner(hostKey, { key: otherKey }) }), /signer key does not match/);
  await assert.rejects(claimRelayMembershipInvite({ ...input, signer: hostSigner(hostKey, { url: 'https://evil.example.com/api/invites/claim' }) }), /not bound to this relay request URL/);
  await assert.rejects(claimRelayMembershipInvite({ ...input, signer: hostSigner(hostKey, { payload: '0'.repeat(64) }) }), /does not cover the request payload/);
  await assert.rejects(claimRelayMembershipInvite({ ...input, signer: hostSigner(hostKey, { now: NOW - 61 }) }), /acceptance window/);
  await assert.rejects(claimRelayMembershipInvite({ ...input, signer: hostSigner(hostKey, { kind: 1 }) }), /non-NIP-98/);
  await assert.rejects(claimRelayMembershipInvite({ ...input, signer: hostSigner(hostKey, { corrupt: true }) }), /signature is invalid/);
  await assert.rejects(claimRelayMembershipInvite({ ...input, signer: hostSigner(hostKey, { raw: () => 'not json' }) }), /malformed NIP-98/);
  assert.equal(fixture.requests.length, 0); // nothing left the module

  // A misbound signer cannot drive the gated query either: only the plain
  // NIP-11 GET ever ran, no POST left the module.
  await assert.rejects(verifyDirectMembershipEvidence({ relay: 'wss://relay.example.com', signer: hostSigner(hostKey, { key: otherKey }), transport: fixture.transport, now: NOW }), /signer key does not match/);
  assert.deepEqual(fixture.requests.map(request => request.method), ['GET']);

  // Forged rosters: a kind 13534 listing the host with an invented role is
  // only evidence when signed by the relay self key.
  const hostPubkey = getPublicKey(hostKey), attackerKey = generateSecretKey();
  const honest = rosterEvent(relayKey, [[hostPubkey, 'member']], NOW);
  const forged = [
    rosterEvent(attackerKey, [[hostPubkey, 'owner']], NOW), // wrong signer
    { ...honest, tags: [['-'], ['member', hostPubkey, 'owner']] }, // relay-key identity, forged content and signature
    'not an event',
  ];
  const forgedFixture = scriptedRelay({ relayKey, enforced: true, members: [[hostPubkey, 'member']], roster: forged });
  const evidence = await verifyDirectMembershipEvidence({ relay: 'wss://relay.example.com', signer: hostSigner(hostKey), transport: forgedFixture.transport, now: NOW });
  assert.equal(evidence.roster.status, 'unavailable');
  assert.equal(evidence.roster.role, undefined);
  assert.equal(evidence.status, 'live-verified-member'); // via the gate, never via the forged list
});

test('enforced denial, open relay and unknown relay are never reported live-verified', async () => {
  const relayKey = generateSecretKey(), hostKey = generateSecretKey();
  const hostPubkey = getPublicKey(hostKey);

  const denied = scriptedRelay({ relayKey, enforced: true }); // host is not a member
  const deniedEvidence = await verifyDirectMembershipEvidence({ relay: 'wss://relay.example.com', signer: hostSigner(hostKey), transport: denied.transport, now: NOW });
  assert.equal(deniedEvidence.liveAdmission, 'denied');
  assert.equal(deniedEvidence.status, 'denied');

  // 401 auth failure and a foreign 403 are not membership denial.
  const unauthorized = scriptedRelay({ relayKey, enforced: true, members: [[hostPubkey, 'member']], queryOutcome: () => ({ status: 401, bodyText: JSON.stringify({ error: 'unauthorized' }) }) });
  assert.equal((await verifyDirectMembershipEvidence({ relay: 'wss://relay.example.com', signer: hostSigner(hostKey), transport: unauthorized.transport, now: NOW })).liveAdmission, 'unavailable');
  const foreign = scriptedRelay({ relayKey, enforced: true, members: [[hostPubkey, 'member']], queryOutcome: () => ({ status: 403, bodyText: JSON.stringify({ error: 'rate_limited' }) }) });
  assert.equal((await verifyDirectMembershipEvidence({ relay: 'wss://relay.example.com', signer: hostSigner(hostKey), transport: foreign.transport, now: NOW })).liveAdmission, 'unavailable');

  // Open relay (self advertised, NIP-43 absent): the gate serves anyone, so a
  // 200 read is roster evidence at most, never live verification.
  const open = scriptedRelay({ relayKey, enforced: false, roster: [rosterEvent(relayKey, [[hostPubkey, 'member']], NOW)] });
  const openEvidence = await verifyDirectMembershipEvidence({ relay: 'wss://relay.example.com', signer: hostSigner(hostKey), transport: open.transport, now: NOW });
  assert.equal(openEvidence.membershipEnforced, false);
  assert.equal(openEvidence.liveAdmission, 'not-checked');
  assert.equal(openEvidence.status, 'roster-member');
  const openCommitted = scriptedRelay({ relayKey, enforced: false, roster: [] });
  const committedEvidence = await verifyDirectMembershipEvidence({ relay: 'wss://relay.example.com', signer: hostSigner(hostKey), transport: openCommitted.transport, now: NOW });
  assert.equal(committedEvidence.liveAdmission, 'not-checked');
  assert.equal(committedEvidence.status, 'unknown'); // no claim supplied, nothing gated

  // No self key advertised: nothing to verify against, unknown.
  const noSelf = scriptedRelay({ relayKey, enforced: false, self: null });
  const noSelfEvidence = await verifyDirectMembershipEvidence({ relay: 'wss://relay.example.com', signer: hostSigner(hostKey), transport: noSelf.transport, now: NOW });
  assert.equal(noSelfEvidence.discovery, 'observed');
  assert.equal(noSelfEvidence.relaySelf, null);
  assert.deepEqual(noSelfEvidence.roster, { status: 'not-checked', lagging: false });
  assert.equal(noSelfEvidence.status, 'unknown');
  assert.deepEqual(noSelf.requests.map(request => request.url), ['https://relay.example.com/']); // no gated query was attempted
});

test('claim responses are validated and the invite code never appears in surfaced errors', async () => {
  const relayKey = generateSecretKey(), hostKey = generateSecretKey();
  const code = 'v2.super-secret-fixture-code';
  const input = { relay: 'wss://relay.example.com', signer: hostSigner(hostKey), inviteCode: code, now: NOW };

  const invalid = scriptedRelay({ relayKey, enforced: true, claimOutcome: () => ({ status: 403, bodyText: JSON.stringify({ error: 'invite_invalid' }) }) });
  await assert.rejects(claimRelayMembershipInvite({ ...input, transport: invalid.transport }), /invite_invalid/);

  // A hostile relay echoing the invite code back in its error field is coarsened.
  const hostile = scriptedRelay({ relayKey, enforced: true, claimOutcome: () => ({ status: 403, bodyText: JSON.stringify({ error: `code ${code} is wrong` }) }) });
  let hostileError: Error | undefined;
  try { await claimRelayMembershipInvite({ ...input, transport: hostile.transport }); assert.fail('expected rejection'); }
  catch (error) { hostileError = error as Error; }
  assert.match(hostileError.message, /unspecified relay error/);
  assert.ok(!hostileError.message.includes(code), 'invite code must not surface in errors');

  for (const bodyText of [
    '{"status":"nope","community_id":"c","host":"relay.example.com","role":"member"}', // invalid status
    '{"status":"joined","community_id":"c","role":"member"}', // missing host
    '{"status":"joined","community_id":"c","host":"evil.example.com","role":"member"}', // different relay host
    'not json',
  ]) {
    const malformed = scriptedRelay({ relayKey, enforced: true, claimOutcome: () => ({ status: 200, bodyText }) });
    let error: Error | undefined;
    try { await claimRelayMembershipInvite({ ...input, transport: malformed.transport }); assert.fail('expected rejection'); }
    catch (caught) { error = caught as Error; }
    assert.ok(!error.message.includes(code), 'invite code must not surface in errors');
  }
});

test('requests are bounded: timeout, external abort and redirects are refused truthfully', async () => {
  const relayKey = generateSecretKey(), hostKey = generateSecretKey();
  const input = { relay: 'wss://relay.example.com', signer: hostSigner(hostKey), inviteCode: 'v2.fixture-code', now: NOW };
  const hang: RelayHttpTransport = async () => await new Promise(() => {}); // ignores the signal: the module must bound it itself

  let timeout: Error | undefined;
  try { await claimRelayMembershipInvite({ ...input, transport: hang, timeoutMs: 40 }); assert.fail('expected rejection'); }
  catch (caught) { timeout = caught as Error; }
  assert.match(timeout.message, /commit status unknown \(Relay HTTP request timeout\)/);
  assert.ok(!timeout.message.includes(input.inviteCode));

  // An abort that already fired before the request leaves must reject promptly,
  // not hang on a transport that ignores the signal.
  const preAborted = new AbortController();
  preAborted.abort();
  let abortedEarly: Error | undefined;
  try { await claimRelayMembershipInvite({ ...input, transport: hang, signal: preAborted.signal, timeoutMs: 5000 }); assert.fail('expected rejection'); }
  catch (caught) { abortedEarly = caught as Error; }
  assert.match(abortedEarly.message, /Relay HTTP request cancelled/);
  assert.ok(!abortedEarly.message.includes(input.inviteCode));

  const controller = new AbortController();
  setTimeout(() => controller.abort(), 10).unref?.();
  await assert.rejects(claimRelayMembershipInvite({ ...input, transport: hang, signal: controller.signal, timeoutMs: 5000 }), /Relay HTTP request cancelled/);

  const redirect: RelayHttpTransport = async () => ({ status: 302, bodyText: '' });
  await assert.rejects(claimRelayMembershipInvite({ ...input, transport: redirect }), /redirect refused/); // authorization is never forwarded across a redirect
  await assert.rejects(discoverRelayMembershipRequirements({ relay: 'wss://relay.example.com', transport: redirect }), /redirect refused/);
});

test('real loopback HTTP journey through fetchRelayTransport with server-side NIP-98 verification', async () => {
  const relayKey = generateSecretKey(), hostKey = generateSecretKey();
  const hostPubkey = getPublicKey(hostKey);
  const relaySelf = getPublicKey(relayKey);
  const members = new Map<string, string>();
  const roster = rosterEvent(relayKey, [[hostPubkey, 'member']], NOW - 60);
  const verified: string[] = [];
  const server = createServer((request, response) => {
    const chunks: Buffer[] = [];
    request.on('data', (chunk: Buffer) => chunks.push(chunk));
    request.on('end', () => {
      const body = Buffer.concat(chunks).toString('utf8');
      const hostHeader = request.headers.host ?? '';
      const send = (status: number, bodyText: string, contentType = 'application/json') => { response.writeHead(status, { 'content-type': contentType }); response.end(bodyText); };
      if (request.method === 'GET' && request.url === '/') { send(200, JSON.stringify({ name: 'loopback-fixture', supported_nips: [1, 43], self: relaySelf }), 'application/nostr+json'); return; }
      if (request.method === 'GET' && request.url === '/redirect') { response.writeHead(302, { location: '/' }); response.end(); return; }
      if (request.method === 'GET' && request.url === '/oversized') { response.writeHead(200, { 'content-length': '300000' }); response.end('x'.repeat(300000)); return; }
      try {
        const pubkey = verifyNip98ServerSide(request.headers.authorization, `http://${hostHeader}${request.url}`, body);
        verified.push(pubkey);
        if (request.url === '/api/invites/claim') {
          const wasMember = members.has(pubkey);
          members.set(pubkey, 'member');
          send(200, JSON.stringify({ status: wasMember ? 'already_member' : 'joined', community_id: COMMUNITY_ID, host: hostHeader, role: 'member' }));
          return;
        }
        if (request.url === '/query') {
          if (!members.has(pubkey)) { send(403, JSON.stringify({ error: 'relay_membership_required' })); return; }
          send(200, JSON.stringify([roster]));
          return;
        }
        send(404, JSON.stringify({ error: 'not_found' }));
      } catch (error) {
        send(401, JSON.stringify({ error: `unauthorized: ${(error as Error).message}` }));
      }
    });
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  assert.ok(address && typeof address === 'object');
  const relay = `http://127.0.0.1:${address.port}`;
  try {
    const claim = await claimRelayMembershipInvite({ relay, signer: hostSigner(hostKey), inviteCode: 'v2.fixture-code', now: NOW });
    assert.equal(claim.status, 'joined');
    assert.equal(claim.host, `127.0.0.1:${address.port}`);
    assert.equal((await claimRelayMembershipInvite({ relay, signer: hostSigner(hostKey), inviteCode: 'v2.fixture-code', now: NOW })).status, 'already_member');
    const evidence = await verifyDirectMembershipEvidence({ relay, signer: hostSigner(hostKey), claim, now: NOW }); // default fetchRelayTransport
    assert.equal(evidence.status, 'live-verified-member');
    assert.equal(evidence.liveAdmission, 'verified');
    assert.deepEqual(evidence.roster, { status: 'observed', role: 'member', createdAt: roster.created_at, lagging: false });
    assert.deepEqual(verified, [hostPubkey, hostPubkey, hostPubkey]); // claim + both queries verified server-side
    const transport = fetchRelayTransport();
    await assert.rejects(transport({ url: `${relay}/redirect`, method: 'GET', headers: {}, body: undefined, timeoutMs: 2000, signal: new AbortController().signal }));
    await assert.rejects(transport({ url: `${relay}/oversized`, method: 'GET', headers: {}, body: undefined, timeoutMs: 2000, signal: new AbortController().signal }), /bounded size/);
  } finally {
    server.closeAllConnections();
    server.close();
    await once(server, 'close');
  }
});
