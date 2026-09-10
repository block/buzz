import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, realpathSync, readFileSync, rmSync, rmdirSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { provision, bindingFingerprint } from '../src/host.ts';
import { migrateSlots, addHarnessBinding, addConversationBinding, installationSlots } from '../src/slots.ts';
import { createGenesis } from '../src/assignment.ts';
import { newKey, publicKey } from '../src/protocol.ts';
import { readPrivate, writePrivate } from '../src/storage.ts';
import { terminal } from './local-terminal.ts';

test('normal binding local transaction refuses lock/stale/authority/active inputs; declined wizard and all old references unchanged', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-conversion-')));
  const ownerSecret = newKey(), agentSecret = newKey(), agent = publicKey(agentSecret);
  const setup = { host: 'conversion', ownerSecret, agentSecret, mode: 'goose' as const, runner: realpathSync(process.execPath), args: ['acp'], workspace: dir, serviceHome: dir, configDirectory: dir, gooseProvider: 'fixture', gooseModels: ['a'] };
  const conversation = { executable: setup.runner, relay: 'ws://127.0.0.1:19490', replyTool: { executable: setup.runner } };
  try {
    provision(dir, setup, createGenesis(publicKey(ownerSecret), agent, setup.host)); migrateSlots(dir);
    const { host: _h, ownerSecret: _o, agentSecret: _a, ...harness } = setup;
    addHarnessBinding(dir, 'B', harness);
    const source = { id: 'B', fingerprint: bindingFingerprint(setup) };
    const manifestPath = join(dir, 'setup.json'), journalPath = join(dir, 'journal.json');
    const original = readFileSync(manifestPath), journal = readFileSync(journalPath);
    mkdirSync(join(dir, 'host.lock'), { mode: 0o700 });
    assert.throws(() => addConversationBinding(dir, agent, source, 'normal', conversation), /EEXIST/);
    assert.equal(statSync(join(dir, 'host.lock')).mode & 0o777, 0o700);
    rmdirSync(join(dir, 'host.lock'));
    assert.throws(() => addConversationBinding(dir, agent, { ...source, fingerprint: '0'.repeat(64) }, 'normal', conversation), /definition changed/);
    const state = readPrivate(journalPath) as any;
    writePrivate(journalPath, { ...state, phase: 'running' });
    assert.throws(() => addConversationBinding(dir, agent, source, 'normal', conversation), /stopped/);
    writePrivate(journalPath, state);
    assert.deepEqual(readFileSync(manifestPath), original);
    const output = await terminal(['local-setup', dir], [
      { prompt: 'Local action [', answer: 'normal' },
      { prompt: 'Existing agent public key: ', answer: agent },
      { prompt: 'NEW immutable normal binding ID: ', answer: 'declined' },
      { prompt: 'Absolute installed buzz-acp executable: ', answer: setup.runner },
      { prompt: 'Buzz CONVERSATION relay URL', answer: conversation.relay },
      { prompt: 'Absolute installed buzz CLI executable: ', answer: setup.runner },
      { prompt: 'Save NEW normal binding under installation lock', answer: 'no' },
    ]);
    assert.ok(!output.includes(agentSecret) && !output.includes(ownerSecret));
    assert.deepEqual(readFileSync(manifestPath), original);
    addConversationBinding(dir, agent, source, 'normal', conversation);
    const converted = readFileSync(manifestPath);
    assert.deepEqual(readFileSync(journalPath), journal);
    const entries = installationSlots(dir);
    assert.equal(entries[0]!.setup.conversation, undefined);
    assert.equal(entries[0]!.bindings.B!.conversation, undefined);
    assert.deepEqual(entries[0]!.bindings.normal!.conversation, conversation);
    assert.equal(bindingFingerprint(entries[0]!.bindings.B!), source.fingerprint);
    assert.throws(() => addConversationBinding(dir, agent, source, 'other', { ...conversation, relay: 'ws://127.0.0.1:19491' }), /authority already pinned/);
    assert.throws(() => addHarnessBinding(dir, 'other', { ...harness, conversation: { ...conversation, relay: 'ws://127.0.0.1:19491' } }), /conversation authority/);
    assert.throws(() => addConversationBinding(dir, agent, source, 'normal', conversation), /Binding exists/);
    assert.deepEqual(readFileSync(manifestPath), converted);
    const identity = join(dir, 'owner.json'), genesisFile = join(dir, 'genesis.json'), standby = join(dir, 'standby');
    writePrivate(identity, { secret: ownerSecret }); writePrivate(genesisFile, state.assignment.genesis);
    const imported = await terminal(['setup', standby, identity], [
      { prompt: 'Host name: ', answer: 'standby' }, { prompt: 'Setup [', answer: '3' },
      { prompt: 'Absolute installed Goose executable (runs acp): ', answer: setup.runner },
      { prompt: 'Allowed workspace (absolute directory): ', answer: dir },
      { prompt: 'Additional allowed workspace (blank for none): ', answer: '' },
      { prompt: 'Locally configured Goose provider ID: ', answer: 'fixture' },
      { prompt: 'Operator-approved compatible exact model IDs', answer: 'a' },
      { prompt: 'Purpose [', answer: '' },
      { prompt: 'Absolute installed buzz-acp executable: ', answer: setup.runner },
      { prompt: 'Buzz CONVERSATION relay URL', answer: conversation.relay },
      { prompt: 'Absolute installed buzz CLI executable: ', answer: setup.runner },
      { prompt: 'Create a NEW agent identity', answer: 'import-standby' },
      { prompt: 'Local public genesis file (no key): ', answer: genesisFile },
      { prompt: `Import exact ${agent}`, answer: 'yes' },
      { prompt: 'Agent private key (64 hex; hidden, never passed as an argument): ', answer: agentSecret },
      { prompt: 'Add another independent NEW agent', answer: 'no' },
    ], process.platform === 'darwin');
    assert.ok(!imported.includes(agentSecret) && !imported.includes(ownerSecret));
    const standbyEntry = installationSlots(standby)[0]!;
    assert.equal(standbyEntry.agent, agent);
    assert.deepEqual(standbyEntry.setup.conversation, conversation);
    assert.deepEqual((readPrivate(standbyEntry.path) as any).assignment, state.assignment);
    assert.equal(statSync(join(standby, 'setup.json')).mode & 0o777, 0o600);
    const corrupted = readPrivate(manifestPath) as any;
    corrupted.setups.normal.conversation.relay = 'ws://127.0.0.1:19491'; writePrivate(manifestPath, corrupted);
    assert.throws(() => installationSlots(dir), /conversation authority/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
