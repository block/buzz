import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { provisionCredentialSlot, orphanCredentialSlots } from '../src/credential-slots.ts';
import { installationSlots, removeSlotKey, importSlotKey, addSlot, reconcileSlot, migrateSlots } from '../src/slots.ts';
import { setupOwner, validateSetup, initialState, provision } from '../src/host.ts';
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

test('explicit exact first-provision recovery verifies orphan key; v3 additions preserve siblings and standby genesis', async () => {
  const { reconcileCredentialProvision } = await import('../src/credential-slots.ts');
  const { addSlot } = await import('../src/slots.ts');
  for (const failure of ['before-store', 'after-store'] as const) {
    const f = fixture();
    try {
      assert.throws(() => provisionCredentialSlot(f.directory, f.setup, f.secret, f.genesis, {
        ...f.backend, create(ref, secret) { if (failure === 'after-store') f.backend.create(ref, secret); throw Error('injected store boundary'); },
      }), /injected/);
      const path = join(f.directory, 'agents', f.agent, 'journal.json'), journal = readFileSync(path, 'utf8');
      assert.throws(() => reconcileCredentialProvision(f.directory, f.setup, newKey(), f.genesis, f.backend), /mismatch/);
      reconcileCredentialProvision(f.directory, f.setup, f.secret, f.genesis, f.backend);
      assert.equal(readFileSync(path, 'utf8'), journal);
      const second = newKey(), key = publicKey(second), root = createGenesis(publicKey(f.owner), key, 'another-host');
      addSlot(f.directory, second, root, 'default', undefined, f.backend);
      assert.equal(installationSlots(f.directory, f.backend).length, 2);
      assert.equal((readPrivate(join(f.directory, 'agents', key, 'journal.json')) as any).assignment.assignedHost, 'another-host');
      assert.equal(readFileSync(path, 'utf8'), journal);
      const manifest = readFileSync(join(f.directory, 'setup.json'), 'utf8');
      for (const secret of [f.secret, f.owner, second]) assert.ok(!manifest.includes(secret));
      removeSlotKey(f.directory, f.agent, f.backend);
      assert.throws(() => reconcileCredentialProvision(f.directory, f.setup, f.secret, f.genesis, f.backend), /Active installation/);
      assert.throws(() => addSlot(f.directory, f.secret, f.genesis, 'default', undefined, f.backend), /Slot exists/);
      assert.equal(installationSlots(f.directory, f.backend).find(s => s.agent === f.agent)!.keyPresent, false);
    } finally { rmSync(f.directory, { recursive: true, force: true }); }
  }
});

test('explicit added-slot recovery reconciles interrupted additions without reset or key replacement', () => {
  for (const failure of ['before-store', 'after-store'] as const) {
    const f = fixture();
    try {
      provisionCredentialSlot(f.directory, f.setup, f.secret, f.genesis, f.backend);
      const siblingJournal = readFileSync(join(f.directory, 'agents', f.agent, 'journal.json'), 'utf8');
      const second = newKey(), key = publicKey(second);
      const standby = createGenesis(publicKey(f.owner), key, 'retained-source');
      assert.throws(() => addSlot(f.directory, second, standby, 'default', undefined, {
        ...f.backend, create(ref, secret) { if (failure === 'after-store') f.backend.create(ref, secret); throw Error('injected store boundary'); },
      }), /injected/);
      const orphanPath = join(f.directory, 'agents', key, 'journal.json');
      assert.ok(existsSync(orphanPath), 'interrupted add retains an inert orphan journal');
      const orphanJournal = readFileSync(orphanPath, 'utf8'), manifestBefore = readFileSync(join(f.directory, 'setup.json'), 'utf8');
      const discovered = orphanCredentialSlots(f.directory);
      assert.equal(discovered.orphans.length, 1);
      assert.equal(discovered.host, f.setup.host);
      assert.equal(discovered.ownerPublic, publicKey(f.owner));
      const found = discovered.orphans[0]!;
      assert.equal(found.agent, key);
      assert.deepEqual(found.candidates.map(c => c.setupId), ['default']);
      assert.equal(found.genesis.initialHost, 'retained-source');
      const fingerprint = found.candidates[0]!.fingerprint;
      // Every wrong input refuses before any store or manifest mutation.
      assert.throws(() => reconcileSlot(f.directory, 'default', second, standby, '0'.repeat(64), f.backend), /Binding definition changed/);
      assert.throws(() => reconcileSlot(f.directory, 'missing', second, standby, undefined, f.backend), /Unknown host harness setup/);
      assert.throws(() => reconcileSlot(f.directory, 'default', second, createGenesis(publicKey(f.owner), key, 'another-host'), fingerprint, f.backend), /differs from exact inert state/);
      // A wrong key refuses against the supplied identity before any store or journal effect.
      assert.throws(() => reconcileSlot(f.directory, 'default', newKey(), standby, fingerprint, f.backend), /Genesis ownership mismatch/);
      assert.throws(() => reconcileSlot(f.directory, 'default', second, createGenesis(publicKey(newKey()), key, 'retained-source'), fingerprint, f.backend), /Genesis ownership mismatch/);
      assert.equal(readFileSync(orphanPath, 'utf8'), orphanJournal);
      assert.equal(readFileSync(join(f.directory, 'setup.json'), 'utf8'), manifestBefore, 'refusals leave the manifest inert');
      // Matching reconcile: an existing orphan key is adopted, a missing one is created exactly once.
      reconcileSlot(f.directory, 'default', second, standby, fingerprint, failure === 'after-store'
        ? { ...f.backend, create() { throw Error('must adopt the existing orphan key'); } }
        : f.backend);
      assert.equal(readFileSync(orphanPath, 'utf8'), orphanJournal, 'recovery never rewrites the retained journal');
      assert.equal(readFileSync(join(f.directory, 'agents', f.agent, 'journal.json'), 'utf8'), siblingJournal, 'sibling journals are untouched');
      const loaded = installationSlots(f.directory, f.backend);
      assert.equal(loaded.length, 2);
      const added = loaded.find(s => s.agent === key)!;
      assert.equal(added.setupId, 'default');
      assert.equal(added.keyPresent, true);
      assert.equal(added.setup.agentSecret, second);
      assert.equal((readPrivate(orphanPath) as any).assignment.assignedHost, 'retained-source');
      const manifest = readFileSync(join(f.directory, 'setup.json'), 'utf8');
      for (const secret of [f.secret, f.owner, second]) assert.ok(!manifest.includes(secret), 'manifest must not persist secrets');
      // Active and public-only retained slots never enter recovery again.
      assert.throws(() => reconcileSlot(f.directory, 'default', second, standby, fingerprint, f.backend), /Retained slot exists; use explicit import-agent-key/);
      removeSlotKey(f.directory, key, f.backend);
      assert.throws(() => reconcileSlot(f.directory, 'default', second, standby, fingerprint, f.backend), /Retained slot exists; use explicit import-agent-key/);
      importSlotKey(f.directory, key, second, f.backend);
    } finally { rmSync(f.directory, { recursive: true, force: true }); }
  }
});

test('added-slot discovery is public-only and refuses foreign or absent partial state', () => {
  const f = fixture();
  try {
    assert.throws(() => orphanCredentialSlots(f.directory), /ENOENT/);
    provisionCredentialSlot(f.directory, f.setup, f.secret, f.genesis, f.backend);
    assert.deepEqual(orphanCredentialSlots(f.directory).orphans, []);
    const foreign = newKey(), foreignKey = publicKey(foreign), otherOwner = newKey();
    const setup = validateSetup({ host: 'host-a', ownerPublic: publicKey(otherOwner), mode: 'fixture', runner: process.execPath, args: [], workspace: f.directory, agentSecret: foreign });
    writePrivate(join(f.directory, 'agents', foreignKey, 'journal.json'), initialState(setup, createGenesis(publicKey(otherOwner), foreignKey, 'host-a')), true);
    assert.throws(() => orphanCredentialSlots(f.directory), /not owned by this installation/);
  } finally { rmSync(f.directory, { recursive: true, force: true }); }
});

test('fresh-assignment recovery, canonical genesis equality and v3-only wrapper', () => {
  const f = fixture();
  try {
    provisionCredentialSlot(f.directory, f.setup, f.secret, f.genesis, f.backend);
    const second = newKey(), key = publicKey(second), fresh = createGenesis(publicKey(f.owner), key, f.setup.host);
    assert.throws(() => addSlot(f.directory, second, fresh, 'default', undefined, { ...f.backend, create() { throw Error('injected'); } }), /injected/);
    const found = orphanCredentialSlots(f.directory).orphans[0]!;
    assert.equal(found.genesis.initialHost, f.setup.host);
    reconcileSlot(f.directory, 'default', second, fresh, found.candidates[0]!.fingerprint, f.backend);
    assert.equal((readPrivate(join(f.directory, 'agents', key, 'journal.json')) as any).assignment.assignedHost, f.setup.host);
    // A semantically identical genesis with reordered fields reconciles the same journal.
    const third = newKey(), thirdKey = publicKey(third);
    const root = createGenesis(publicKey(f.owner), thirdKey, f.setup.host);
    assert.throws(() => addSlot(f.directory, third, root, 'default', undefined, { ...f.backend, create() { throw Error('injected'); } }), /injected/);
    const reordered = { initialHost: root.initialHost, agent: root.agent, owner: root.owner, id: root.id, v: 1 as const };
    reconcileSlot(f.directory, 'default', third, reordered, orphanCredentialSlots(f.directory).orphans[0]!.candidates[0]!.fingerprint, f.backend);
    assert.equal((readPrivate(join(f.directory, 'agents', thirdKey, 'journal.json')) as any).assignment.assignedHost, f.setup.host);
    assert.equal(installationSlots(f.directory, f.backend).length, 3);
  } finally { rmSync(f.directory, { recursive: true, force: true }); }
  const directory = mkdtempSync(join(tmpdir(), 'beehive-legacy-reconcile-'));
  try {
    const owner = newKey(), secret = newKey();
    const setup = validateSetup({ host: 'legacy', ownerSecret: owner, agentSecret: secret, mode: 'fixture', runner: process.execPath, args: [], workspace: directory });
    provision(directory, setup, createGenesis(publicKey(owner), publicKey(secret), 'legacy'));
    migrateSlots(directory);
    assert.throws(() => reconcileSlot(directory, 'default', secret, createGenesis(publicKey(owner), publicKey(secret), 'legacy'), undefined, memoryCredentials()), /migrate-slots/);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
