import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, realpathSync, rmSync, existsSync, readFileSync, writeFileSync, symlinkSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { codexHomePaths, prepareCodexHome } from '../src/codex-home.ts';
import { runtimeBindings } from '../src/settings-runtime.ts';
import { newKey, publicKey } from '../src/protocol.ts';
import type { Setup } from '../src/host.ts';
import type { Settings } from '../src/settings.ts';

function fixture(t: test.TestContext) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-codex-home-')));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return root;
}
const settings: Settings = { version: 1, revision: 1, agents: [], providers: [{ id: 'provider', name: 'Fixture', type: 'openai', endpoint: 'https://api.openai.com/v1', key: { service: 'beehive', account: 'provider:00000000-0000-0000-0000-000000000000' } }], runtimes: [{ id: 'runtime', name: 'Fixture', harness: 'codex', cli: '/fixture/codex', executable: '/fixture/codex-acp', providerId: 'provider', model: 'custom-model' }] };

test('base-slot Codex projection is pure, public-slot bound, stable, and preserves legacy paths', t => {
  const root = fixture(t), agent = publicKey(newKey());
  const base: Setup = { mode: 'fixture', host: publicKey(newKey()), ownerPublic: publicKey(newKey()), runner: '/fixture/runner', args: [], workspace: root, serviceHome: root, configDirectory: root };
  // Explicit public slot identity works without an agent key read (v3/key-removed catalogs).
  const first = runtimeBindings(base, settings, agent)['runtime:runtime']!;
  assert.notEqual(first.serviceHome, root); assert.notEqual(first.configDirectory, first.serviceHome);
  assert.equal(existsSync(first.serviceHome!), false);
  assert.deepEqual(runtimeBindings(base, settings, agent)['runtime:runtime'], first);
  const other = runtimeBindings(base, settings, publicKey(newKey()))['runtime:runtime']!;
  assert.notEqual(other.serviceHome, first.serviceHome);
  const next = runtimeBindings(base, { ...settings, runtimes: [{ ...settings.runtimes[0]!, id: 'next' }] }, agent)['runtime:next']!;
  assert.notEqual(next.serviceHome, first.serviceHome);
  const old = runtimeBindings({ ...base, serviceHome: join(root, 'legacy-home'), configDirectory: join(root, 'legacy-config') }, settings, agent)['runtime:runtime']!;
  assert.equal(old.serviceHome, join(root, 'legacy-home')); assert.equal(old.configDirectory, join(root, 'legacy-config'));
  assert.equal(old.codex!.managedHome, undefined);
  assert.throws(() => runtimeBindings(base, settings), /public slot authority/);
});

test('managed directories retain state across preparation and never import unrelated HOME config', t => {
  const parent = fixture(t), value = { parent, identity: 'a'.repeat(64) }, paths = codexHomePaths(value);
  const unrelated = join(parent, 'config.toml'); writeFileSync(unrelated, 'do-not-read-or-retarget', { mode: 0o600 });
  prepareCodexHome(value, paths.home, paths.config);
  for (const path of [paths.root, paths.home, paths.config]) assert.equal(statSync(path).mode & 0o777, 0o700);
  assert.equal(existsSync(join(paths.config, 'config.toml')), false);
  writeFileSync(join(paths.config, 'retained-session'), 'resume-state', { mode: 0o600 });
  prepareCodexHome(value, paths.home, paths.config);
  assert.equal(readFileSync(join(paths.config, 'retained-session'), 'utf8'), 'resume-state');
  assert.equal(readFileSync(unrelated, 'utf8'), 'do-not-read-or-retarget');
  assert.throws(() => prepareCodexHome(value, parent, paths.config), /binding mismatch/);
});

test('unowned, symlinked or partial managed homes fail closed without adoption or deletion', t => {
  const parent = fixture(t);
  for (const [index, kind] of ['missing-marker', 'wrong-marker', 'symlink-root', 'symlink-config'].entries()) {
    const value = { parent, identity: String(index).repeat(64) }, paths = codexHomePaths(value);
    const unrelated = join(parent, `unrelated-${index}`); mkdirSync(unrelated, { mode: 0o700 });
    writeFileSync(join(unrelated, 'sentinel'), 'unchanged');
    if (kind === 'symlink-root') symlinkSync(unrelated, paths.root);
    else if (kind === 'symlink-config') {
      prepareCodexHome(value, paths.home, paths.config);
      rmSync(paths.config, { recursive: true }); symlinkSync(unrelated, paths.config);
    } else {
      mkdirSync(paths.root, { mode: 0o700 });
      if (kind === 'wrong-marker') writeFileSync(join(paths.root, 'beehive-owner.json'), '{}', { mode: 0o600 });
    }
    assert.throws(() => prepareCodexHome(value, paths.home, paths.config));
    assert.equal(readFileSync(join(unrelated, 'sentinel'), 'utf8'), 'unchanged');
    if (kind !== 'symlink-config') assert.equal(existsSync(paths.home), false);
  }
});
