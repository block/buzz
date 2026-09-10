import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createCredential, readCredential, removeCredential, systemCredentials, type CredentialBackend } from '../src/credential-store.ts';
import { bootstrapHostIdentity, readHostIdentity } from '../src/host-identity.ts';
import { newKey } from '../src/protocol.ts';
import { memoryCredentials } from './credential-fixture.ts';

test('explicit isolated credential lifecycle; absence, locked/denied and bad roundtrip fail closed', () => {
  const backend = memoryCredentials(), secret = newKey();
  const reference = createCredential('agent', secret, backend);
  assert.equal(readCredential(reference, backend), secret);
  assert.throws(() => createCredential('agent', secret, backend), /exists/);
  removeCredential(reference, backend);
  assert.throws(() => readCredential(reference, backend), /missing or intentionally removed/);
  for (const reason of ['locked', 'denied', 'unavailable']) {
    const failed: CredentialBackend = { read() { throw Error(reason); }, create() { assert.fail('must not write after unavailable read'); }, remove() { assert.fail('must not delete after unavailable read'); } };
    assert.throws(() => createCredential('host', secret, failed), new RegExp(reason));
    assert.throws(() => removeCredential(reference, failed), new RegExp(reason));
  }
  const bad: CredentialBackend = { read: () => null, create() {}, remove() {} };
  assert.throws(() => createCredential('host', secret, bad), /missing/);
  assert.throws(() => createCredential('owner', secret, systemCredentials), /No plaintext fallback/);
  const again = createCredential('agent', secret, backend);
  const noDelete = { ...backend, remove() {} };
  assert.throws(() => removeCredential(again, noDelete), /not verified/);
});

test('host state persists public references only; removed keys never regenerate; default does not touch OS or write plaintext', () => {
  const root = mkdtempSync(join(tmpdir(), 'beehive-key-reference-'));
  try {
    const backend = memoryCredentials();
    const owner = 'a'.repeat(64);
    assert.throws(() => bootstrapHostIdentity(join(root, 'production'), 'host', owner, 'wss://example.invalid'), /OS credential store unavailable/);
    assert.equal(existsSync(join(root, 'production', 'host-identity.json')), false);
    const directory = join(root, 'fixture');
    const identity = bootstrapHostIdentity(directory, 'host', owner, 'wss://example.invalid', backend);
    const content = readFileSync(join(directory, 'host-identity.json'), 'utf8');
    assert.ok(!content.includes(identity.secret));
    const reference = JSON.parse(content).key;
    removeCredential(reference, backend);
    assert.throws(() => readHostIdentity(directory, backend), /missing or intentionally removed/);
    assert.throws(() => bootstrapHostIdentity(directory, 'host', owner, 'wss://example.invalid', backend), /already exists/);
    assert.equal(readFileSync(join(directory, 'host-identity.json'), 'utf8'), content);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
