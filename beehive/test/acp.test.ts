import { provisionSetup } from './provision.ts';
import { test } from 'node:test';
import { spawnSync } from 'node:child_process';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, rmSync, mkdirSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { AgentSession, prepareAgent, type AgentLaunch } from '../src/acp.ts';
import { host } from '../src/host.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';
import { message, newKey, publicKey, type Message } from '../src/protocol.ts';
import { writePrivate } from '../src/storage.ts';

const fixture = resolve('test/acp-fixture.ts');
function plan(dir: string, mode = 'ok'): AgentLaunch {
  return { executable: realpathSync(process.execPath), args: [fixture, mode], workspace: dir, home: dir, configDirectory: dir, databricksHost: 'https://fixture.invalid', model: 'databricks-claude-haiku-4-5' };
}
async function cleanup(s: AgentSession) { await s.owned.stop(); }
test('external ACP boundary: snapshot, catalog provenance, exact-model same-session evidence and failures', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-acp-')));
  try {
    const input = plan(dir); const prepared = prepareAgent(input);
    (input.args as string[]).push('mutated');
    assert.equal(prepared.plan.args.length, 2);
    assert.equal(Object.hasOwn(prepared.env, 'DATABRICKS_TOKEN'), false);
    assert.equal(Object.hasOwn(prepared.env, 'BUZZ_PRIVATE_KEY'), false);
    assert.throws(() => prepareAgent({ ...plan(dir), databricksHost: 'https://user:secret@example.com' }));
    for (const mode of ['ok', 'empty', 'filtered', 'wrong-model', 'wrong-session', 'cancelled', 'reject', 'malformed', 'timeout', 'flood', 'bad-tail']) {
      const s = new AgentSession(plan(dir, mode), 1500);
      try {
        if (['malformed', 'timeout', 'flood'].includes(mode)) { await assert.rejects(s.catalog()); continue; }
        const catalog = await s.catalog();
        assert.equal(catalog.authentication, 'unverified');
        assert.equal(catalog.state, mode === 'empty' ? 'empty' : mode === 'filtered' ? 'filtered' : 'reported');
        assert.ok(!JSON.stringify(catalog).includes('DO-NOT-RELAY'));
        if (['wrong-model', 'wrong-session', 'cancelled', 'reject', 'bad-tail'].includes(mode)) {
          await assert.rejects(s.verify(), e => e instanceof Error && !e.message.includes('DO-NOT-RELAY'));
        } else {
          const evidence = await s.verify();
          assert.equal(evidence.model, plan(dir).model);
          assert.equal(evidence.session, 'fixture-session');
          assert.match(evidence.responseHash, /^[0-9a-f]{64}$/);
          assert.equal(evidence.executableHash, s.prepared.executableHash);
        }
      } finally { await cleanup(s); }
    }
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('production host Start uses external ACP evidence; Save and Stop do not need auth', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-acp-host-')));
  const hostDir = join(dir, 'host'); mkdirSync(hostDir);
  const ownerSecret = newKey(); const agentSecret = newKey(); const agent = publicKey(agentSecret);
  const p = plan(dir);
  const authDir = join(dir, 'isolated-auth'); mkdirSync(authDir);
  provisionSetup(join(hostDir, 'setup.json'), { host: 'acp-host', ownerSecret, agentSecret, runner: p.executable, args: p.args, workspace: dir, mode: 'buzz-agent-databricks-v2', serviceHome: dir, configDirectory: authDir, databricksHost: p.databricksHost });
  const authInfo = spawnSync(process.execPath, ['src/cli.ts', 'auth-info', hostDir], { encoding: 'utf8', timeout: 3000 });
  assert.equal(authInfo.status, 0); assert.match(authInfo.stdout, /acp-host/); assert.match(authInfo.stdout, /auth databricks/);
  assert.ok(!authInfo.stdout.includes(ownerSecret) && !authInfo.stdout.includes(agentSecret));
  const server = await relay(0, publicKey(ownerSecret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`;
  const h = await host(hostDir, url);
  const seen: Message[] = [];
  const client = connect(url, ownerSecret, m => seen.push(m)); await client.ready;
  async function request(type: 'start' | 'save' | 'stop', revision: number, body = {}) {
    const m = message(type, 'acp-host', agent, revision, body); client.send(m);
    for (let i = 0; i < 200; i++) {
      const r = seen.find(r => r.type === 'receipt' && r.body.operation === m.id);
      if (r) return r;
      await delay(25);
    }
    throw Error('No host receipt');
  }
  try {
    assert.equal((await request('save', 0, { model: p.model, workspace: dir, profile: 'default' })).body.result, 'saved; running configuration unchanged');
    assert.equal((await request('start', 1)).body.result, 'accepted');
    const state = JSON.parse(readFileSync(join(hostDir, 'journal.json'), 'utf8'));
    assert.equal(state.actual.evidence.model, p.model);
    assert.equal(state.actual.evidence.session, 'fixture-session');
    assert.equal(state.actual.selection.model, p.model);
    assert.equal(state.actual.executableHash, state.actual.evidence.executableHash);
    rmSync(authDir, { recursive: true });
    assert.equal((await request('save', 2, { model: p.model, workspace: dir, profile: 'default' })).body.result, 'saved; running configuration unchanged');
    // The auth directory is already absent; both Save and Stop remain usable.
    assert.equal((await request('stop', 3)).body.result, 'accepted');
    const stopped = JSON.parse(readFileSync(join(hostDir, 'journal.json'), 'utf8'));
    assert.equal(stopped.binding.host, 'acp-host'); assert.equal(stopped.phase, 'stopped');
    assert.ok(!readFileSync(join(dir, 'relay.json'), 'utf8').includes('fixture-session'));
  } finally {
    await h.close(); client.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
  }
});
