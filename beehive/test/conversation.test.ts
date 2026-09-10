import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, rmSync, readFileSync, existsSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { prepareConversation, conversationSummary } from '../src/conversation.ts';
import { host } from '../src/host.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';
import { message, newKey, publicKey, type Message } from '../src/protocol.ts';
import { writePrivate } from '../src/storage.ts';

const model = 'databricks-claude-haiku-4-5';
const executable = realpathSync(process.execPath);
function agent(dir: string) { return { executable, args: [resolve('test/acp-fixture.ts')], workspace: dir, home: dir, configDirectory: dir, databricksHost: 'https://fixture.invalid', model }; }

test('external buzz-acp launch contract preserves identity and rejects lossy/unsafe transport inputs', () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-conversation-')));
  try {
    const secret = newKey(); const owner = publicKey(newKey());
    const setup = { executable, relay: 'wss://conversation.invalid', authTag: 'fixture-opaque-credential' };
    const p = prepareConversation(setup, agent(dir), secret, owner);
    assert.equal(p.agentPublicKey, publicKey(secret));
    assert.equal(p.env.BUZZ_PRIVATE_KEY, secret);
    assert.equal(p.env.BUZZ_ACP_AGENT_OWNER, owner);
    assert.equal(p.env.BUZZ_AUTH_TAG, setup.authTag);
    assert.equal(p.env.BUZZ_ACP_MODEL, model);
    assert.equal(p.env.BUZZ_AGENT_MODEL, model);
    assert.equal(p.env.BUZZ_ACP_RESPOND_TO, 'owner-only');
    assert.equal(p.env.BUZZ_ACP_AGENT_COMMAND, executable);
    assert.equal(p.env.BUZZ_ACP_AGENT_ARGS, agent(dir).args.join(','));
    assert.equal(p.env.BUZZ_RELAY_URL, 'wss://conversation.invalid/');
    assert.equal(Object.hasOwn(p.env, 'DATABRICKS_TOKEN'), false);
    assert.equal(Object.hasOwn(p.env, 'BUZZ_ACP_REQUIRED_MODEL'), false);
    assert.equal(Object.isFrozen(p.env), true);
    const second = prepareConversation(setup, agent(dir), secret, owner);
    assert.equal(second.agentPublicKey, p.agentPublicKey);
    for (const relay of ['ws://example.com', 'wss://user:password@example.com', 'https://example.com', 'wss://example.com/?secret=x']) {
      assert.throws(() => prepareConversation({ ...setup, relay }, agent(dir), secret, owner));
    }
    for (const args of [['a,b'], [''], ['a\0b']]) assert.throws(() => prepareConversation(setup, { ...agent(dir), args }, secret, owner));
    const summary = JSON.stringify(conversationSummary(setup));
    for (const value of [secret, setup.authTag, executable]) assert.ok(!summary.includes(value));
    assert.match(summary, /unverified/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('real management WS rejects unsafe conversation Start before spawn; Save/Stop and key binding survive', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-conversation-host-')));
  const ownerSecret = newKey(); const agentSecret = newKey(); const key = publicKey(agentSecret);
  const setup = { host: 'conversation-host', ownerSecret, agentSecret, runner: executable, args: agent(dir).args, workspace: dir, mode: 'buzz-agent-databricks-v2', serviceHome: dir, configDirectory: dir, databricksHost: 'https://fixture.invalid', conversation: { executable, relay: 'wss://conversation.invalid', authTag: 'fixture-attestation' } };
  writePrivate(join(dir, 'setup.json'), setup);
  const server = await relay(0, publicKey(ownerSecret), join(dir, 'relay.json'));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`;
  const h = await host(dir, url); const seen: Message[] = [];
  const client = connect(url, ownerSecret, m => seen.push(m)); await client.ready;
  async function request(type: 'start' | 'save' | 'stop', revision: number, body = {}) {
    const m = message(type, setup.host, key, revision, body); client.send(m);
    for (let i = 0; i < 160; i++) {
      const r = seen.find(r => r.type === 'receipt' && r.body.operation === m.id);
      if (r) return r;
      await delay(25);
    }
    throw Error('No management receipt');
  }
  try {
    const start = await request('start', 0);
    assert.match(String(start.body.result), /Conversation Start unavailable.*No process started/);
    const state = JSON.parse(readFileSync(join(dir, 'journal.json'), 'utf8'));
    assert.equal(state.phase, 'stopped'); assert.equal(state.actual, null); assert.equal(state.revision, 0);
    assert.equal((await request('save', 0, { model, workspace: dir, profile: 'default' })).body.result, 'saved; running configuration unchanged');
    assert.equal((await request('stop', 1)).body.result, 'accepted');
    assert.equal(JSON.parse(readFileSync(join(dir, 'setup.json'), 'utf8')).agentSecret, agentSecret);
    assert.ok(!JSON.stringify(seen).includes('fixture-attestation'));
    assert.ok(!JSON.stringify(seen).includes(agentSecret));
    const cli = spawnSync(process.execPath, ['src/cli.ts', 'conversation-setup', dir], { encoding: 'utf8', timeout: 3000 });
    assert.notEqual(cli.status, 0); assert.ok(existsSync(join(dir, 'host.lock')));
  } finally {
    await h.close(); client.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve()));
  }
  try {
    // Input/preflight failure unwinds only this invocation's local exclusion.
    const cli = spawnSync(process.execPath, ['src/cli.ts', 'conversation-setup', dir], { input: '/nonexistent\n', encoding: 'utf8', timeout: 3000 });
    assert.notEqual(cli.status, 0); assert.equal(existsSync(join(dir, 'host.lock')), false);
    assert.equal(JSON.parse(readFileSync(join(dir, 'setup.json'), 'utf8')).agentSecret, agentSecret);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
