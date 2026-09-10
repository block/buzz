import test from 'node:test';
import assert from 'node:assert/strict';
import { nativeCredentials, type NativeEntry } from '../src/native-credentials.ts';
import { createCredential, removeCredential, readCredential } from '../src/credential-store.ts';
import { newKey } from '../src/protocol.ts';

test('native bridge production seam uses only Beehive per-key entries; absence differs from errors; no secret error logs', () => {
  const entries = new Map<string, string>();
  const calls: string[] = [];
  let failure = false;
  class Entry implements NativeEntry {
    id: string;
    constructor(service: string, account: string) { assert.equal(service, 'beehive'); assert.match(account, /^(agent|host|owner):[0-9a-f]{64}$/); this.id = account; calls.push(account); }
    getPassword() { if (failure) throw Error('synthetic sensitive native detail'); return entries.get(this.id) ?? null; }
    setPassword(secret: string) { entries.set(this.id, secret); }
    deleteCredential() { return entries.delete(this.id); }
  }
  const backend = nativeCredentials(() => Entry), secret = newKey();
  const reference = createCredential('agent', secret, backend);
  assert.equal(readCredential(reference, backend), secret);
  failure = true;
  assert.throws(() => readCredential(reference, backend), error => {
    assert.ok(error instanceof Error); assert.match(error.message, /locked, denied, unavailable/);
    assert.ok(!error.message.includes('synthetic sensitive')); assert.equal(error.cause, undefined); return true;
  });
  failure = false;
  removeCredential(reference, backend);
  assert.equal(backend.read(reference), null);
  assert.ok(calls.length > 3, 'read-back and absence checks reach the native entry seam');
  assert.throws(() => backend.read({ ...reference, service: 'buzz-desktop' } as any), /operation failed/);
});
