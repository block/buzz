import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, mkdirSync, writeFileSync, readFileSync, rmSync, chmodSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { AgentSession, prepareAgent, type AgentLaunch } from '../src/acp.ts';
import { codexModels } from '../src/codex.ts';

function fixture() {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-codex-')));
  const home = join(dir, 'service-home'); mkdirSync(home, { mode: 0o700 });
  const configDirectory = join(dir, 'agent-config'); mkdirSync(configDirectory, { mode: 0o700 });
  const apiKeyFile = join(dir, 'key'); writeFileSync(apiKeyFile, 'fixture-codex-private-key', { mode: 0o600 });
  const executable = join(dir, 'codex-acp');
  writeFileSync(executable, `#!${process.execPath}\nawait import(${JSON.stringify(pathToFileURL(resolve('test/conversation-harness-fixture.ts')).href)});\n`, { mode: 0o700 });
  writeFileSync(join(dir, 'mode'), 'ok');
  const launch: AgentLaunch = { executable, args: [], workspace: dir, home, configDirectory, databricksHost: '', harness: 'codex', codex: { cli: realpathSync(process.execPath), apiKeyFile, models: ['codex-model-a'] }, model: 'codex-model-a', instructions: 'CODEX NATIVE PROFILE' };
  return { dir, launch, apiKeyFile };
}

test('Codex private service env, missing/unsafe credential and adapter arguments fail closed', () => {
  const { dir, launch, apiKeyFile } = fixture();
  try {
    const prepared = prepareAgent(launch);
    assert.deepEqual(Object.keys(prepared.env).sort(), ['CODEX_CONFIG', 'CODEX_HOME', 'HOME', 'OPENAI_API_KEY', 'PATH']);
    assert.equal(prepared.env.HOME, launch.home);
    assert.equal(prepared.env.CODEX_HOME, launch.configDirectory);
    assert.deepEqual(JSON.parse(prepared.env.CODEX_CONFIG!), { model: launch.model });
    assert.throws(() => prepareAgent({ ...launch, model: 'unapproved' }), /approved model/);
    assert.throws(() => prepareAgent({ ...launch, configDirectory: launch.home }), /distinct HOME/);
    assert.ok('cliExecutableHash' in prepared && typeof prepared.cliExecutableHash === 'string');
    assert.match(prepared.cliExecutableHash, /^[a-f0-9]{64}$/);
    assert.throws(() => prepareAgent({ ...launch, args: ['acp'] }), /zero-argument/);
    chmodSync(apiKeyFile, 0o644);
    assert.throws(() => prepareAgent(launch), /Codex API key prerequisite/);
    rmSync(apiKeyFile);
    assert.throws(() => prepareAgent(launch), /Codex API key prerequisite/);
    assert.throws(() => codexModels({ models: { currentModelId: 'wrong', availableModels: [] } }, launch.model), /evidence/);
    assert.throws(() => codexModels({ models: { currentModelId: launch.model, availableModels: [{ modelId: 42 }] } }, launch.model), /advertised/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

for (const mode of ['ok', 'wrong-model', 'missing-native', 'wrong-protocol', 'missing-model', 'auth-rejected', 'codex-drift']) test(`Codex native ACP prerequisite ${mode}, no invented switch or Goose method`, async () => {
  const { dir, launch } = fixture();
  writeFileSync(join(dir, 'mode'), mode);
  const session = new AgentSession(launch);
  try {
    if (mode === 'ok' || mode === 'codex-drift') {
      assert.equal((await session.catalog()).authentication, 'unverified');
      if (mode === 'codex-drift') await assert.rejects(session.verify());
      else {
        const evidence = await session.verify();
        assert.equal(evidence.model, launch.model); assert.match(evidence.session, /^codex-/);
        assert.equal(readFileSync(join(dir, 'received-system-instructions'), 'utf8'), launch.instructions);
      }
    } else {
      await assert.rejects(session.catalog());
      await assert.rejects(session.verify());
      assert.ok(!readFileSync(join(dir, 'codex-rpc-methods'), 'utf8').includes('session/prompt'), 'reject capability/model/auth before any prompt');
    }
    const methods = readFileSync(join(dir, 'codex-rpc-methods'), 'utf8');
    assert.ok(!methods.includes('session/set_model')); assert.ok(!methods.includes('_goose'));
  } finally {
    session.cancel(); await session.owned.stop();
    const pid = Number(readFileSync(join(dir, 'descendant-pid'), 'utf8'));
    assert.throws(() => process.kill(pid, 0), (e: any) => e.code === 'ESRCH');
    rmSync(dir, { recursive: true, force: true });
  }
});


test('Codex OS provider binding requires the resolved exact-reference key and forbids file fallback', () => {
  const { dir, launch } = fixture();
  try {
    const codex = { cli: launch.codex!.cli, models: [launch.model], credential: { service: 'beehive' as const, account: 'provider:00000000-0000-0000-0000-000000000000' } };
    assert.throws(() => prepareAgent({...launch,codex}),/OS provider credential unavailable/);
    assert.throws(() => prepareAgent({...launch,codex,resolvedProviderKey:'bad key'}),/OS provider credential unavailable/);
    assert.throws(() => prepareAgent({...launch,codex:{...codex,apiKeyFile:launch.codex!.apiKeyFile},resolvedProviderKey:'synthetic'}),/Invalid local Codex/);
    const prepared = prepareAgent({...launch,codex,resolvedProviderKey:'synthetic-os-only'});
    assert.equal(prepared.env.OPENAI_API_KEY,'synthetic-os-only');
    assert.deepEqual(JSON.parse(prepared.env.CODEX_CONFIG!),{model:launch.model});
    assert.ok(!JSON.stringify(codex).includes('synthetic-os-only'));
  } finally { rmSync(dir,{recursive:true,force:true}); }
});
