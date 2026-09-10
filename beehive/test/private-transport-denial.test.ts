import test from 'node:test';
import assert from 'node:assert/strict';
import { newKey, publicKey } from '../src/protocol.ts';
import { privateHostTransport } from '../src/host-transport.ts';
import { hostPairing } from '../src/host-registration.ts';
import { nostrFixture } from './nostr-fixture.ts';

test('actual AUTH, REQ and publication refusals fail private startup without preflight', async () => {
  const key = newKey(), owner = newKey();
  const faults = { open: true, denyAuth: false, denyPublication: false, denyReq: false };
  const relay = await nostrFixture(publicKey(owner), new Set(), faults);
  const pairing = hostPairing({ version: 1, purpose: 'beehive-host-registration', host: publicKey(key), owner: publicKey(owner), label: 'fixture', relay: relay.url, nonce: newKey() });
  try {
    for (const field of ['denyAuth', 'denyPublication', 'denyReq'] as const) {
      faults[field] = true;
      const transport = privateHostTransport(pairing, key);
      const wire = transport.connect(relay.url, '', () => {});
      try { await assert.rejects(wire.ready, field === 'denyAuth' ? /authentication denied/ : field === 'denyReq' ? /closed subscription/ : /rejected publication/); }
      finally { wire.close(); faults[field] = false; }
    }
    assert.equal(relay.history.length, 0);
    assert.equal(relay.httpRequests.length, 0);
  } finally { await relay.close(); }
});
