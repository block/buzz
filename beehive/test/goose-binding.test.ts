import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, realpathSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { terminal } from './local-terminal.ts';
import { installationSlots } from '../src/slots.ts';
import { host } from '../src/host.ts';
import { relay } from '../src/relay.ts';
import { newKey, publicKey } from '../src/protocol.ts';
import { writePrivate, readPrivate } from '../src/storage.ts';

test('actual local add-Goose binding -> remote TUI selects compatible model -> explicit Restart executes native ACP, same identity and Y', { timeout: 30000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-goose-binding-'))), source = join(dir, 'source'), home = join(dir, 'goose-home'); mkdirSync(home, { mode: 0o700 });
  const secret = newKey(), identity = join(dir, 'owner.json'); writePrivate(identity, { secret });
  const runner = join(dir, 'goose-fixture.ts');
  writeFileSync(runner, `#!${realpathSync(process.execPath)}\nif (process.argv[2] !== 'acp') throw Error('Expected goose acp');\nawait import(${JSON.stringify(pathToFileURL(resolve('test/conversation-harness-fixture.ts')).href)});\n`, { mode: 0o700 });
  writeFileSync(join(dir, 'mode'), 'ok');
  await terminal(['setup', source, identity], [
    { prompt: 'Host name: ', answer: 'source' }, { prompt: 'Setup [', answer: '1' },
    { prompt: 'Absolute fixture runner executable: ', answer: realpathSync(process.execPath) },
    { prompt: 'Allowed workspace (absolute directory): ', answer: dir },
    { prompt: 'Additional allowed workspace (blank for none): ', answer: '' },
    { prompt: 'Absolute fixture TypeScript script: ', answer: resolve('test/runner.ts') },
    { prompt: 'Create a NEW agent identity', answer: 'yes' },
    { prompt: 'Add another independent NEW agent', answer: 'yes' },
    { prompt: 'Add another independent NEW agent', answer: 'no' },
  ]);
  const local = await terminal(['setup', source], [
    { prompt: 'Local action [', answer: 'add-goose' },
    { prompt: 'Existing binding ID to reuse: ', answer: 'default' },
    { prompt: 'NEW immutable Goose binding ID: ', answer: 'Goose' },
    { prompt: 'Absolute installed Goose executable (runs acp): ', answer: runner },
    { prompt: 'Allowed workspace (absolute directory): ', answer: dir },
    { prompt: 'Existing dedicated service HOME (not Desktop HOME): ', answer: home },
    { prompt: 'Locally configured Goose provider ID: ', answer: 'fixture-provider' },
    { prompt: 'Operator-approved compatible exact model IDs (comma-separated): ', answer: 'goose-a,goose-b' },
    { prompt: 'Save NEW Goose binding only (no identity/selection/restart)? [yes/no]: ', answer: 'yes' },
  ]);
  assert.match(local, /not authentication/);
  const [X, Y] = installationSlots(source); assert.ok(X && Y);
  const state = () => readPrivate(X.path) as any;
  const manifest = readFileSync(join(source, 'setup.json')), sibling = readFileSync(Y.path);
  const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string'); const url = `ws://127.0.0.1:${address.port}`;
  const h = await host(source, url);
  let before: any;
  try {
    const output = await terminal(['tui', identity, url], [
      { prompt: 'beehive> ', answer: 'hosts' },
      { prompt: 'beehive> ', answer: 'select 1' },
      { prompt: 'beehive> ', answer: 'start', observedRevision: 0 },
      { prompt: 'beehive> ', answer: 'binding Goose', observedRevision: 1, gate: () => { if (state().phase !== 'running') return false; before = state().actual; return true; } },
      { prompt: 'beehive> ', answer: 'save', observedRevision: 2, gate: () => state().selected.harnessSetup?.id === 'Goose' },
      { prompt: 'Model: ', answer: 'goose-b' }, { prompt: 'Workspace: ', answer: dir }, { prompt: 'Behavior profile: ', answer: 'default' },
      { prompt: 'beehive> ', answer: 'restart', observedRevision: 3, gate: () => { assert.deepEqual(state().actual, before); return true; } },
      { prompt: 'beehive> ', answer: 'show', observedRevision: 4, gate: () => state().actual?.evidence?.model === 'goose-b' },
      { prompt: 'beehive> ', answer: 'stop', observedRevision: 4 },
      { prompt: 'beehive> ', answer: 'quit', observedRevision: 5 },
    ]);
    assert.match(output, /Allowed models: \["goose-a","goose-b"\]/);
    assert.match(output, /external-acp-session/);
    assert.deepEqual(state().runs[before.run], before);
    const applied = Object.values(state().runs).find((r: any) => r.harnessSetup.id === 'Goose') as any;
    assert.equal(applied.evidence.model, 'goose-b'); assert.equal(applied.selection.model, 'goose-b');
    assert.equal(state().binding.agent, X.agent); assert.equal(state().phase, 'stopped');
    assert.deepEqual(readFileSync(Y.path), sibling); assert.deepEqual(readFileSync(join(source, 'setup.json')), manifest);
    for (const key of [secret, X.setup.agentSecret!, Y.setup.agentSecret!]) assert.ok(!output.includes(key) && !local.includes(key));
  } finally {
    await h.close(); for (const c of server.clients) c.terminate(); await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
  }
});
