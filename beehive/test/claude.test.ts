import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, mkdirSync, writeFileSync, readFileSync, rmSync, chmodSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { AgentSession, prepareAgent, type AgentLaunch } from '../src/acp.ts';
import { claudeModels } from '../src/claude.ts';

function fixture() {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-claude-')));
  const home = join(dir, 'service-home'); mkdirSync(home, { mode: 0o700 });
  const apiKeyFile = join(dir, 'key'); writeFileSync(apiKeyFile, 'fixture-claude-private-key', { mode: 0o600 });
  const executable = join(dir, 'claude-agent-acp');
  writeFileSync(executable, `#!${process.execPath}\nawait import(${JSON.stringify(pathToFileURL(resolve('test/conversation-harness-fixture.ts')).href)});\n`, { mode: 0o700 });
  writeFileSync(join(dir, 'mode'), 'ok');
  const launch: AgentLaunch = { executable, args: [], workspace: dir, home, configDirectory: home, databricksHost: '', harness: 'claude', claude: { cli: realpathSync(process.execPath), apiKeyFile, models: ['claude-model-a'] }, model: 'claude-model-a', instructions: 'CLAUDE NATIVE PROFILE' };
  return { dir, launch, apiKeyFile };
}

test('Claude private service env, missing/unsafe credential and adapter arguments fail closed', () => {
  const { dir, launch, apiKeyFile } = fixture();
  try {
    const prepared = prepareAgent(launch);
    assert.deepEqual(Object.keys(prepared.env).sort(), ['ANTHROPIC_API_KEY', 'ANTHROPIC_MODEL', 'CLAUDE_CODE_EXECUTABLE', 'HOME', 'PATH']);
    assert.equal(prepared.env.HOME, launch.home);
    assert.ok('cliExecutableHash' in prepared && typeof prepared.cliExecutableHash === 'string');
    assert.match(prepared.cliExecutableHash, /^[a-f0-9]{64}$/);
    assert.throws(() => prepareAgent({ ...launch, args: ['acp'] }), /zero-argument/);
    chmodSync(apiKeyFile, 0o644);
    assert.throws(() => prepareAgent(launch), /Claude API key prerequisite/);
    rmSync(apiKeyFile);
    assert.throws(() => prepareAgent(launch), /Claude API key prerequisite/);
    assert.throws(() => claudeModels({ models: { currentModelId: 'wrong', availableModels: [] } }, launch.model), /evidence/);
    assert.throws(() => claudeModels({ models: { currentModelId: launch.model, availableModels: [{ modelId: 42 }] } }, launch.model), /advertised/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

for (const mode of ['ok', 'wrong-model', 'missing-native', 'auth-rejected', 'claude-drift']) test(`Claude native ACP prerequisite ${mode}, no invented switch or Goose method`, async () => {
  const { dir, launch } = fixture();
  writeFileSync(join(dir, 'mode'), mode);
  const session = new AgentSession(launch);
  try {
    if (mode === 'ok' || mode === 'claude-drift') {
      assert.equal((await session.catalog()).authentication, 'unverified');
      if (mode === 'claude-drift') await assert.rejects(session.verify());
      else {
        const evidence = await session.verify();
        assert.equal(evidence.model, launch.model); assert.match(evidence.session, /^claude-/);
        assert.equal(readFileSync(join(dir, 'received-system-instructions'), 'utf8'), launch.instructions);
      }
    } else await assert.rejects(session.catalog());
    const methods = readFileSync(join(dir, 'claude-rpc-methods'), 'utf8');
    assert.ok(!methods.includes('session/set_model')); assert.ok(!methods.includes('_goose'));
  } finally {
    session.cancel(); await session.owned.stop();
    const pid = Number(readFileSync(join(dir, 'descendant-pid'), 'utf8'));
    assert.throws(() => process.kill(pid, 0), (e: any) => e.code === 'ESRCH');
    rmSync(dir, { recursive: true, force: true });
  }
});
