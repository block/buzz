import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { provisionCredentialSlot } from '../src/credential-slots.ts';
import { installationSlots, removeSlotKey, importSlotKey } from '../src/slots.ts';
import { setupOwner, validateSetup } from '../src/host.ts';
import { createGenesis } from '../src/assignment.ts';
import { newKey, publicKey } from '../src/protocol.ts';
import { memoryCredentials } from './credential-fixture.ts';
import { readPrivate, writePrivate } from '../src/storage.ts';

function fixture() {
  const directory = mkdtempSync(join(tmpdir(), 'beehive-credential-slots-'));
  const owner = newKey(), secret = newKey(), agent = publicKey(secret), backend = memoryCredentials();
  const setup = validateSetup({ host: 'host-a', ownerPublic: publicKey(owner), mode: 'fixture', runner: process.execPath, args: [], workspace: directory });
  const genesis = createGenesis(publicKey(owner), agent, setup.host);
  return { directory, owner, secret, agent, backend, setup, genesis };
}

test('owner-public provisioning, retained deletion/retry and explicit matching import never persist identity secrets', () => {
  const f = fixture();
  try {
    provisionCredentialSlot(f.directory, f.setup, f.secret, f.genesis, f.backend);
    const manifest = readFileSync(join(f.directory, 'setup.json'), 'utf8');
    const path = join(f.directory, 'agents', f.agent, 'journal.json'), journal = readFileSync(path, 'utf8');
    assert.ok(!manifest.includes(f.secret) && !manifest.includes(f.owner));
    assert.ok(!journal.includes(f.secret) && !journal.includes(f.owner));
    const loaded = installationSlots(f.directory, f.backend)[0]!;
    assert.equal(loaded.setup.ownerSecret, undefined);
    assert.equal(setupOwner(loaded.setup), publicKey(f.owner));
    assert.equal(loaded.setup.agentSecret, f.secret);
    removeSlotKey(f.directory, f.agent, f.backend);
    removeSlotKey(f.directory, f.agent, f.backend);
    const removed = installationSlots(f.directory, f.backend)[0]!;
    assert.equal(removed.keyPresent, false);
    assert.equal(removed.setup.agentSecret, undefined);
    assert.throws(() => importSlotKey(f.directory, f.agent, newKey(), f.backend), /does not match/);
    importSlotKey(f.directory, f.agent, f.secret, f.backend);
    assert.equal(installationSlots(f.directory, f.backend)[0]!.setup.agentSecret, f.secret);
    assert.equal(readFileSync(path, 'utf8'), journal);
    assert.equal(readFileSync(join(f.directory, 'setup.json'), 'utf8'), manifest);
  } finally { rmSync(f.directory, { recursive: true, force: true }); }
});

test('preserved-state validation precedes deletion; unavailable differs from missing; partial creation never resets', () => {
  const f = fixture();
  try {
    provisionCredentialSlot(f.directory, f.setup, f.secret, f.genesis, f.backend);
    const unavailable = { ...f.backend, read() { throw Error('locked: unlock explicitly'); } };
    assert.throws(() => installationSlots(f.directory, unavailable), /locked/);
    assert.throws(() => removeSlotKey(f.directory, f.agent, unavailable), /locked/);
    const path = join(f.directory, 'agents', f.agent, 'journal.json');
    const state = readPrivate(path) as any;
    state.binding.owner = publicKey(newKey()); writePrivate(path, state);
    assert.throws(() => removeSlotKey(f.directory, f.agent, { ...f.backend, remove() { assert.fail('must validate before OS delete'); } }), /mismatch/);
    assert.throws(() => provisionCredentialSlot(f.directory, f.setup, f.secret, f.genesis, f.backend), /refusing identity reset/);
  } finally { rmSync(f.directory, { recursive: true, force: true }); }
  const partial = fixture();
  try {
    assert.throws(() => provisionCredentialSlot(partial.directory, partial.setup, partial.secret, partial.genesis, { ...partial.backend, create() { throw Error('denied'); } }), /denied/);
    assert.throws(() => installationSlots(partial.directory, partial.backend), /ENOENT/);
    assert.throws(() => provisionCredentialSlot(partial.directory, partial.setup, partial.secret, partial.genesis, partial.backend), /partial provision/);
  } finally { rmSync(partial.directory, { recursive: true, force: true }); }
});

test('public owner validation rejects absent and conflicting identity', () => {
  const owner = newKey();
  assert.throws(() => setupOwner({}), /required/);
  assert.throws(() => setupOwner({ ownerPublic: 'bad' }), /Invalid/);
  assert.throws(() => setupOwner({ ownerPublic: publicKey(newKey()), ownerSecret: owner }), /Conflicting/);
});
