import test from 'node:test';
import assert from 'node:assert/strict';
import { verifyEvent } from 'nostr-tools/pure';
import { membershipSigner } from '../src/relay-admission.ts';
import { verifyDirectMembershipEvidence } from '../src/direct-membership.ts';
import { newKey, publicKey } from '../src/protocol.ts';
import { nostrFixture } from './nostr-fixture.ts';

test('same-second production HTTP signers use fresh IDs; fixture refuses real replay', async t => {
  const now = Date.now(); t.mock.method(Date, 'now', () => now);
  const secret = newKey(), signer = membershipSigner(secret);
  const request = { url: 'https://example.invalid/query', method: 'POST' as const, payloadSha256Hex: 'a'.repeat(64) };
  const first = JSON.parse(await signer.signHttpAuthentication(request));
  const second = JSON.parse(await membershipSigner(secret).signHttpAuthentication(request));
  assert.notEqual(first.id, second.id); assert.equal(first.created_at, second.created_at);
  for (const event of [first, second]) {
    assert.ok(verifyEvent(event)); assert.equal(event.pubkey, publicKey(secret));
    assert.equal(event.kind, 27235); assert.equal(event.content, '');
    assert.deepEqual(event.tags.slice(0, 3), [['u', request.url], ['method', 'POST'], ['payload', request.payloadSha256Hex]]);
    assert.equal(event.tags[3][0], 'nonce');
  }
  const relay = await nostrFixture(publicKey(secret), new Set([publicKey(secret)]));
  let signed: string | undefined;
  const replaySigner = { hostPublicKey: signer.hostPublicKey, async signHttpAuthentication(input: typeof request) { signed ??= await signer.signHttpAuthentication(input); return signed; } };
  try {
    assert.equal((await verifyDirectMembershipEvidence({ relay: relay.url, signer: replaySigner })).status, 'live-verified-member');
    assert.equal((await verifyDirectMembershipEvidence({ relay: relay.url, signer: replaySigner })).status, 'unknown');
    assert.equal((await verifyDirectMembershipEvidence({ relay: relay.url, signer: membershipSigner(secret) })).status, 'live-verified-member');
  } finally { await relay.close(); }
});
