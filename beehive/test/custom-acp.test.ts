import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, writeFileSync, readFileSync, rmSync, mkdirSync, rmdirSync, statSync, existsSync, chmodSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { discoverPresets, presets } from '../src/presets.ts';
import { validateCustom, readCustom, type CustomAcp } from '../src/custom-acp.ts';
import { validateSetup, bindingFingerprint, host } from '../src/host.ts';
import { migrateSlots, installationSlots, addHarnessBinding, retireHarnessBinding, addSlot } from '../src/slots.ts';
import { writePrivate } from '../src/storage.ts';
import { newKey, publicKey, message, type Message } from '../src/protocol.ts';
import { createGenesis } from '../src/assignment.ts';
import { provisionSetup } from './provision.ts';
import { terminal } from './local-terminal.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';

const definition = (dir: string): CustomAcp => ({ id: 'custom-fixture', label: 'Local custom diagnostic', executable: join(dir, 'not-installed'), args: ['literal,comma', '$(echo not-a-shell)', ''], env: { LOCAL_ONLY: 'private-marker' }, installHint: 'Manual local setup', installInstructionsUrl: 'https://example.invalid/docs', contract: 'diagnostic' });

test('ten pinned presets distinguish executable/CLI/adapter absence without launching or authenticating', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-presets-')));
  try {
    assert.deepEqual(presets.map(p => [p.id, p.command, [...p.args]]), [
      ['pi','pi-acp',[]], ['devin','devin',['acp']], ['cursor','cursor-agent',['acp']], ['omp','omp',['acp']], ['grok','grok',['agent','--always-approve','stdio']], ['opencode','opencode',['acp']], ['kimi','kimi',['acp']], ['amp','amp-acp',[]], ['hermes','hermes-acp',[]], ['openclaw','openclaw',['acp']],
    ]);
    assert.deepEqual(presets.filter(p => 'underlyingCli' in p).map(p => p.id), ['pi', 'amp']);
    const pi = () => discoverPresets(dir).find(p => p.id === 'pi')!;
    assert.equal(pi().availability, 'not-installed');
    writeFileSync(join(dir, 'pi'), '#!/bin/sh\nexit 99\n', { mode: 0o700 });
    assert.equal(pi().availability, 'adapter-missing');
    writeFileSync(join(dir, 'pi-acp'), `#!/bin/sh\ntouch '${join(dir, 'unexpected-execution')}'\n`, { mode: 0o700 });
    assert.equal(pi().availability, 'available');
    rmSync(join(dir, 'pi')); assert.equal(pi().availability, 'cli-missing');
    for (const p of discoverPresets(dir)) { assert.equal(p.authentication, 'unverified'); assert.deepEqual(p.env, {}); assert.match(p.reason, /no exact actual-session model/); }
    assert.equal(existsSync(join(dir, 'unexpected-execution')), false);
    const output = await terminal(['presets'], []);
    for (const p of presets) assert.ok(output.includes(p.label));
    assert.match(output, /NotApplicable auth is not authenticated/);
    assert.match(output, /Gateway/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('custom schema is owner-local structured data, fails closed for unknown contracts/fields/reserved env and unsafe files', () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-custom-schema-')));
  try {
    const valid = definition(dir);
    assert.deepEqual(validateCustom(valid), valid);
    for (const bad of [null, { ...valid, contract: 'protocol2' }, { ...valid, capability: 'systemPrompt' }, { ...valid, installScript: 'curl | sh' }, { ...valid, args: 'acp' }, { ...valid, args: ['a\0b'] }, { ...valid, executable: 'relative' }, { ...valid, id: '../escape' }, { ...valid, env: { BUZZ_PRIVATE_KEY: 'override' } }, { ...valid, env: { HOME: dir } }, { ...valid, env: { GOOSE_MODEL: 'wrong' } }, { ...valid, env: { NODE_OPTIONS: '--require injected' } }, { ...valid, env: { OK: 1 } }, { ...valid, label: '\x1b[31m' }, { ...valid, installInstructionsUrl: 'https://secret@example.invalid/' }]) assert.throws(() => validateCustom(bad));
    const file = join(dir, 'definition.json'); writePrivate(file, valid);
    assert.deepEqual(readCustom(file), valid);
    writeFileSync(join(dir, 'public.json'), JSON.stringify(valid), { mode: 0o644 });
    chmodSync(join(dir, 'public.json'), 0o644);
    assert.throws(() => readCustom(join(dir, 'public.json')), /owner-only/);
    writePrivate(file, { ...valid, contract: 'goose-native' });
    const setup = { host: 'h', ownerSecret: newKey(), mode: 'diagnostic-acp', custom: valid, runner: valid.executable, args: valid.args, workspace: dir };
    assert.throws(() => validateSetup({ ...setup, conversation: {} }), /Diagnostic/);
    assert.throws(() => validateSetup({ ...setup, args: ['changed'] }), /mismatch/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('actual local custom/preset wizard creates immutable diagnostic bindings; remote selection cannot execute or inject; retirement stays inspectable', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-custom-wizard-'))), installation = join(dir, 'host');
  const owner = newKey(), agent = newKey();
  const initial = { host: 'local', ownerSecret: owner, agentSecret: agent, mode: 'fixture', runner: realpathSync(process.execPath), args: [resolve('test/fixture.ts')], workspace: dir };
  provisionSetup(join(installation, 'setup.json'), initial); migrateSlots(installation);
  const entry = installationSlots(installation)[0]!, journal = readFileSync(entry.path), file = join(dir, 'custom.json');
  writePrivate(file, definition(dir));
  let runtime: Awaited<ReturnType<typeof host>> | undefined;
  const r = await relay(0, publicKey(owner), join(dir, 'relay.json'));
  const address = r.address(); assert.ok(address && typeof address !== 'string'); const url = `ws://127.0.0.1:${address.port}`;
  let client: ReturnType<typeof connect> | undefined;
  try {
    const steps = [
      { prompt: 'Local action [', answer: 'add-custom' }, { prompt: 'Existing binding ID to reuse: ', answer: 'default' },
      { prompt: 'Absolute owner-only custom definition JSON file: ', answer: file }, { prompt: 'Allowed workspace', answer: dir },
      { prompt: 'NEW immutable binding ID: ', answer: 'custom' }, { prompt: 'Save NEW binding only', answer: 'yes' },
    ];
    const beforeWizard = readFileSync(join(installation, 'setup.json'));
    writeFileSync(file, '{"env":{"PRIVATE":"do-not-print-malformed-secret"', { mode: 0o600 });
    await assert.rejects(terminal(['local-setup', installation], steps.slice(0,3)), error => {
      assert.match(String(error), /Malformed custom JSON/); assert.ok(!String(error).includes('do-not-print-malformed-secret')); return true;
    });
    assert.deepEqual(readFileSync(join(installation, 'setup.json')), beforeWizard);
    writePrivate(file, definition(dir));
    const output = await terminal(['local-setup', installation], steps);
    assert.match(output, /Start unavailable/); assert.ok(!output.includes('private-marker'));
    await terminal(['local-setup', installation], [
      { prompt: 'Local action [', answer: 'add-preset' }, { prompt: 'Existing binding ID', answer: 'default' },
      { prompt: 'Preset ID: ', answer: 'devin' },
      // Ambient PATH has no vendor in this fresh controlled CLI context; no executable is launched.
      ...(!discoverPresets().find(p => p.id === 'devin')!.executable ? [{ prompt: 'absolute intended local executable', answer: join(dir, 'devin-not-installed') }] : []),
      { prompt: 'Allowed workspace', answer: dir }, { prompt: 'NEW immutable binding ID: ', answer: 'preset' }, { prompt: 'Save NEW binding only', answer: 'yes' },
    ]);
    const bindings = installationSlots(installation)[0]!.bindings, custom = bindings.custom!;
    assert.deepEqual(readFileSync(entry.path), journal); assert.equal(statSync(join(installation,'setup.json')).mode & 0o777, 0o600);
    const manifest = readFileSync(join(installation,'setup.json'));
    const secret = newKey(); assert.throws(() => addSlot(installation, secret, createGenesis(publicKey(owner), publicKey(secret), 'local'), 'custom'), /Diagnostic-only/);
    mkdirSync(join(installation,'host.lock')); try { assert.throws(() => addHarnessBinding(installation, 'blocked', { ...custom, host: undefined } as never)); } finally { rmdirSync(join(installation,'host.lock')); }
    assert.deepEqual(readFileSync(join(installation,'setup.json')), manifest);
    runtime = await host(installation, url);
    const seen: Message[] = []; client = connect(url, owner, m => seen.push(m)); await client.ready;
    async function wait(predicate: () => boolean) { for (let n=0;n<300;n++) { if (predicate()) return; await delay(20); } throw Error('Missing public receipt/inventory'); }
    await wait(() => seen.some(m => m.type === 'inventory'));
    const inventory = seen.find(m => m.type === 'inventory')!;
    const advertised = (inventory.body.harnessSetups as any[]).find(b => b.id === 'custom');
    assert.equal(advertised.availability, 'diagnostic-only'); assert.deepEqual(advertised.models, []); assert.match(advertised.reason, /Start\/Restart unavailable/);
    for (const hidden of ['private-marker', file, 'not-installed', '$(echo']) assert.ok(!JSON.stringify(inventory).includes(hidden));
    for (const body of [
      { model: 'fixture-model', workspace: dir, profile: 'default', harnessSetup: { id: 'custom', fingerprint: bindingFingerprint(custom) } },
      { model: 'fixture-model', workspace: dir, profile: 'default', env: { EVIL: 'remote' }, args: ['remote'] },
    ]) {
      const request = message('save', 'local', publicKey(agent), inventory.revision, body); client.send(request);
      await wait(() => seen.some(m => m.type === 'receipt' && m.body.operation === request.id));
      assert.notEqual(seen.find(m => m.type === 'receipt' && m.body.operation === request.id)!.body.result, 'accepted');
    }
    assert.deepEqual(readFileSync(join(installation,'setup.json')), manifest);
    client.close(); client = undefined; await runtime.close(); runtime = undefined;
    retireHarnessBinding(installation, { id: 'custom', fingerprint: bindingFingerprint(custom) });
    assert.deepEqual(installationSlots(installation)[0]!.bindings.custom, custom);
    assert.ok(installationSlots(installation)[0]!.retiredBindings?.custom);
  } finally { client?.close(); await runtime?.close(); await new Promise<void>(resolve => r.close(() => resolve())); rmSync(dir, { recursive: true, force: true }); }
});
