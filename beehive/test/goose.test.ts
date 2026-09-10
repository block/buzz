import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, realpathSync, writeFileSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { terminal } from './local-terminal.ts';
import { installationSlots } from '../src/slots.ts';
import { host, bindingFingerprint } from '../src/host.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';
import { newKey, publicKey, message, type Message } from '../src/protocol.ts';
import { writePrivate, readPrivate } from '../src/storage.ts';
import { profileRevision } from '../src/profiles.ts';
import { gooseModels, prepareAgent } from '../src/acp.ts';

test('Goose native model evidence rejects absent, ambiguous, unadvertised and mismatched acknowledgements', () => {
  const value = { configOptions: [{ configId: 'model', category: 'model', currentValue: 'a', options: [{ value: 'a' }] }] };
  assert.deepEqual(gooseModels(value, 'a'), ['a']);
  for (const bad of [{}, { models: { currentModelId: 'a' } }, { configOptions: [...value.configOptions, ...value.configOptions] }, { configOptions: [{ ...value.configOptions[0], options: [] }] }]) assert.throws(() => gooseModels(bad, 'a'));
  assert.throws(() => gooseModels(value, 'b'));
});

test('actual setup entry Goose/new/reuse/hidden standby and selected binding ACP Restart preserve identity/profile/sibling', { timeout: 35000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-goose-'))), source = join(dir, 'source');
  const secret = newKey(), identity = join(dir, 'owner.json'); writePrivate(identity, { secret });
  const runner = join(dir, 'goose-fixture.ts');
  writeFileSync(runner, `#!${realpathSync(process.execPath)}\nif (process.argv[2] !== 'acp') throw Error('Expected goose acp');\nawait import(${JSON.stringify(pathToFileURL(resolve('test/conversation-harness-fixture.ts')).href)});\n`, { mode: 0o700 });
  writeFileSync(join(dir, 'mode'), 'ok');
  const steps = (name: string) => [
    { prompt: 'Host name: ', answer: name }, { prompt: 'Setup [', answer: '3' },
    { prompt: 'Absolute installed Goose executable (runs acp): ', answer: runner },
    { prompt: 'Allowed workspace (absolute directory): ', answer: dir },
    { prompt: 'Additional allowed workspace (blank for none): ', answer: '' },
    { prompt: 'Locally configured Goose provider ID: ', answer: 'fixture-provider' },
    { prompt: 'Operator-approved compatible exact model IDs (comma-separated): ', answer: 'goose-a,goose-b' },
    { prompt: 'Purpose [', answer: 'diagnostic' },
  ];
  let output = await terminal(['setup', source, identity], [...steps('source'),
    { prompt: 'Create a NEW agent identity', answer: 'yes' },
    { prompt: 'Add another independent NEW agent', answer: 'yes' },
    { prompt: 'Add another independent NEW agent', answer: 'no' },
  ]);
  assert.match(output, /Goose owns provider credentials/); assert.ok(!output.includes('auth databricks'));
  assert.match(output, /Start uses an ACP greeting probe, not yet a Buzz relay conversation agent/);
  assert.match(output, /explicit local-setup .*action normal/);
  assert.match(output, /no provider login or community admission has been verified/);
  const slots = installationSlots(source), X = slots[0]!, Y = slots[1]!;
  const state = () => readPrivate(X.path) as any;
  const originalY = readFileSync(Y.path), originalManifest = readFileSync(join(source, 'setup.json'));
  output += await terminal(['setup', source], [{ prompt: 'Local action [', answer: 'reuse' }, { prompt: 'Existing agent public key: ', answer: X.agent }]);
  assert.deepEqual(readFileSync(join(source, 'setup.json')), originalManifest);
  const root = join(dir, 'genesis.json'); writePrivate(root, state().assignment.genesis);
  const target = join(dir, 'target');
  output += await terminal(['setup', target, identity], [...steps('target'),
    { prompt: 'Create a NEW agent identity', answer: 'import-standby' },
    { prompt: 'Local public genesis file (no key): ', answer: root },
    { prompt: `Import exact ${X.agent}`, answer: 'yes' },
    { prompt: 'Agent private key (64 hex; hidden, never passed as an argument): ', answer: X.setup.agentSecret! },
    { prompt: 'Add another independent NEW agent', answer: 'no' },
  ], process.platform === 'darwin');
  for (const key of [secret, X.setup.agentSecret!, Y.setup.agentSecret!]) assert.ok(!output.includes(key));
  for (const path of [join(source, 'setup.json'), join(target, 'setup.json'), X.path]) assert.equal(statSync(path).mode & 0o777, 0o600);
  assert.equal(statSync(join(source, 'service-home')).mode & 0o777, 0o700);
  const plan = prepareAgent({ executable: runner, args: ['acp'], workspace: dir, home: X.setup.serviceHome!, configDirectory: X.setup.serviceHome!, databricksHost: '', harness: 'goose', provider: 'fixture-provider', model: 'goose-a' });
  assert.equal(plan.env.GOOSE_MODEL, 'goose-a'); assert.equal(plan.env.BUZZ_AGENT_CONFIG_DIR, undefined); assert.equal(plan.env.BUZZ_PRIVATE_KEY, undefined);
  const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string'); const url = `ws://127.0.0.1:${address.port}`;
  const h = await host(source, url), standby = await host(target, url), seen: Message[] = [];
  const client = connect(url, secret, m => seen.push(m)); await client.ready;
  async function request(type: 'start' | 'restart' | 'stop' | 'save', body = {}, hostname = 'source') {
    const revision = hostname === 'source' ? state().revision : 0;
    const m = message(type, hostname, X.agent, revision, body); client.send(m);
    for (let n = 0; n < 400; n++) { const receipt = seen.find(e => e.type === 'receipt' && e.body.operation === m.id); if (receipt) return String(receipt.body.result); await delay(20); }
    throw Error(`Missing ${type} receipt`);
  }
  try {
    assert.notEqual(await request('start', {}, 'target'), 'accepted');
    const instructions = 'GOOSE-PROFILE: keep behavior independent of provider.';
    const revision = profileRevision('Goose behavior', null, instructions);
    const selection = { model: 'goose-a', workspace: dir, profile: revision, behavior: { name: 'Goose behavior', parent: null, revision, instructions }, harnessSetup: { id: 'default', fingerprint: bindingFingerprint(X.setup) } };
    client.send(message('profile', 'profiles', 'profiles', 0, selection.behavior));
    assert.match(await request('save', selection), /saved/);
    assert.equal(await request('start'), 'accepted'); const actual = state().actual;
    assert.match(actual.evidence.session, /^goose-/); assert.equal(actual.selection.model, 'goose-a');
    assert.equal(readFileSync(join(dir, 'received-system-instructions'), 'utf8'), instructions);
    assert.match(await request('save', { ...selection, model: 'goose-b' }), /saved/);
    assert.deepEqual(state().actual, actual);
    assert.equal(await request('restart'), 'accepted');
    assert.notEqual(state().actual.evidence.session, actual.evidence.session); assert.equal(state().actual.evidence.model, 'goose-b');
    assert.equal(state().actual.selection.behavior.revision, revision); assert.equal(state().binding.agent, X.agent);
    assert.deepEqual(state().runs[actual.run], actual);
    assert.notEqual(await request('save', { ...selection, model: 'unknown' }), 'saved; running configuration unchanged');
    assert.notEqual(await request('save', { ...selection, harnessSetup: { id: 'unknown', fingerprint: selection.harnessSetup.fingerprint } }), 'saved; running configuration unchanged');
    assert.notEqual(await request('save', { ...selection, harnessSetup: { id: 'default', fingerprint: '0'.repeat(64) } }), 'saved; running configuration unchanged');
    const running = state().actual; writeFileSync(join(dir, 'mode'), 'wrong-model');
    assert.notEqual(await request('restart'), 'accepted'); assert.deepEqual(state().actual, running);
    rmSync(X.setup.serviceHome!, { recursive: true });
    assert.equal(await request('stop'), 'accepted');
    assert.deepEqual(readFileSync(Y.path), originalY); assert.deepEqual(readFileSync(join(source, 'setup.json')), originalManifest);
  } finally {
    await h.close(); await standby.close(); client.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
  }
});
