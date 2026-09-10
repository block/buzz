import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { WebSocketServer } from 'ws';
import { mkdtempSync, realpathSync, writeFileSync, rmSync, existsSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
import { schnorr } from '@noble/curves/secp256k1';
import { setTimeout as delay } from 'node:timers/promises';
import { terminal } from './local-terminal.ts';
import { installationSlots } from '../src/slots.ts';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { relay } from '../src/relay.ts';
import { readPrivate, writePrivate } from '../src/storage.ts';
import { pathToFileURL } from 'node:url';
import { connect } from '../src/client.ts';
import { profileRevision } from '../src/profiles.ts';
import { newKey, publicKey, message, type Message } from '../src/protocol.ts';

// Explicitly opt-in installed executable; never opens an owner profile or provider.
for (const kind of ['goose', 'claude', 'codex', 'custom', 'anthropic', 'openai-compat', 'openrouter', 'databricks_v2', 'databricks-oauth']) for (const converting of (['custom', 'anthropic', 'openai-compat', 'databricks_v2', 'databricks-oauth'].includes(kind) ? [false] : [false, true])) test(`${kind} normal wizard + TUI Start/Restart + host CLI service subprocess + installed CLI signed replies (${converting ? 'selected diagnostic B conversion and immutable replacement' : 'initial normal default'})`, { skip: !process.env.BEEHIVE_REAL_BUZZ_ACP }, async t => {
  const custom = kind === 'custom', goose = kind === 'goose' || custom, claude = kind === 'claude', codex = kind === 'codex';
  const databricks = kind.startsWith('databricks'), oauth = kind === 'databricks-oauth';
  const buzz = databricks || ['anthropic', 'openai-compat', 'openrouter'].includes(kind);
  const provider = databricks ? 'databricks_v2' : kind;
  const endpoint = databricks ? 'https://databricks.fixture.invalid' : `https://${kind}.fixture.invalid/v1`;
  const keyName = databricks ? 'DATABRICKS_TOKEN' : kind === 'anthropic' ? 'ANTHROPIC_API_KEY' : kind === 'openai-compat' ? 'OPENAI_COMPAT_API_KEY' : 'OPENROUTER_API_KEY';
  const title = codex ? 'Codex' : 'Claude';
  const model = goose ? 'goose-model-a' : `${kind}-model-a`;
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-installed-')));
  writeFileSync(join(dir, 'mode'), 'ok');
  const ownerSecret = newKey(); const owner = publicKey(ownerSecret);
  const identity = join(dir, 'owner.json'), installation = join(dir, 'host');
  writePrivate(identity, { secret: ownerSecret });
  const keyFile = join(dir, 'claude-key');
  writeFileSync(keyFile, `fixture-${buzz ? kind : codex ? 'codex' : 'claude'}-private-key`, { mode: 0o600 });
  const initialKey = join(dir, 'initial-anthropic-key');
  if (buzz && converting) writeFileSync(initialKey, 'fixture-anthropic-private-key', { mode: 0o600 });
  const runner = join(dir, codex ? 'codex-acp' : claude ? 'claude-agent-acp' : 'goose');
  writeFileSync(runner, `#!${realpathSync(process.execPath)}\nif (${!goose ? 'process.argv.length !== 2' : "process.argv[2] !== 'acp'"}) throw Error('adapter argv mismatch');\nawait import(${JSON.stringify(pathToFileURL(resolve('test/conversation-harness-fixture.ts')).href)});\n`, { mode: 0o700 });
  const customFile = join(dir, 'custom.json');
  const literal = `$(touch ${join(dir, 'shell-was-invoked')}); a,b`;
  if (custom) {
    writePrivate(customFile, { id: 'fixture-custom', label: 'CUSTOM Goose-native TS fixture (not a vendor)', executable: runner,
      args: ['acp', literal], env: { CUSTOM_PRIVATE: 'owner-only-custom-value' }, installHint: 'Fixture only', installInstructionsUrl: '', contract: 'goose-native' });
    writeFileSync(runner, `#!${realpathSync(process.execPath)}\nif (process.argv.length !== 4 || process.argv[2] !== 'acp' || process.argv[3] !== ${JSON.stringify(literal)} || process.env.CUSTOM_PRIVATE !== 'owner-only-custom-value') throw Error('custom argv/env mismatch');\nawait import(${JSON.stringify(pathToFileURL(resolve('test/conversation-harness-fixture.ts')).href)});\n`, { mode: 0o700 });
  }
  const http = createServer();
  t.after(async () => { http.closeAllConnections(); if (http.listening) await new Promise<void>(resolve => http.close(() => resolve())); rmSync(dir, { recursive: true, force: true }); });
  await new Promise<void>(resolve => http.listen(0, '127.0.0.1', resolve));
  const address = http.address(); assert.ok(address && typeof address !== 'string');
  const setupOutput = await terminal(['setup', installation, identity], [
    { prompt: 'Host name: ', answer: 'journey' }, { prompt: 'Setup [', answer: buzz ? '8' : custom ? '6' : codex ? '5' : claude ? '4' : '3' },
    ...(custom ? [{ prompt: 'Absolute owner-only custom definition JSON file: ', answer: customFile }] : [{ prompt: buzz ? 'Absolute installed buzz-agent executable: ' : codex ? 'Absolute installed codex-acp adapter: ' : claude ? 'Absolute installed claude-agent-acp adapter: ' : 'Absolute installed Goose executable (runs acp): ', answer: runner }]),
    ...(!goose && !buzz ? [
      { prompt: `Absolute installed ${kind} CLI: `, answer: realpathSync(process.execPath) },
      { prompt: `Absolute owner-only local ${codex ? 'OPENAI' : 'ANTHROPIC'}_API_KEY file`, answer: keyFile },
      { prompt: `Operator-approved compatible exact ${title} model IDs`, answer: model },
    ] : []),
    ...(buzz ? [
      { prompt: 'Buzz Agent provider [', answer: converting ? 'anthropic' : provider },
      ...(databricks ? [{ prompt: 'Databricks authentication [', answer: oauth ? 'external-oauth' : 'token' }] : []),
      ...(!oauth ? [{ prompt: `Absolute owner-only local ${converting ? 'ANTHROPIC_API_KEY' : keyName} file`, answer: converting ? initialKey : keyFile }] : []),
      { prompt: 'Provider HTTPS base URL', answer: converting ? 'https://anthropic.fixture.invalid/v1' : endpoint },
      ...(kind === 'openai-compat' ? [{ prompt: 'OpenAI-compatible wire [', answer: 'responses' }] : []),
      { prompt: 'Operator-approved compatible exact Buzz Agent model IDs', answer: model },
    ] : []),
    { prompt: 'Allowed workspace (absolute directory): ', answer: dir },
    { prompt: 'Additional allowed workspace (blank for none): ', answer: '' },
    ...(goose ? [{ prompt: 'Locally configured Goose provider ID: ', answer: 'fixture-provider' },
    { prompt: 'Operator-approved compatible exact model IDs (comma-separated): ', answer: 'goose-model-a' }] : []),
    { prompt: 'Purpose [', answer: converting ? 'diagnostic' : '' },
    ...(!converting ? [
    { prompt: 'Absolute installed buzz-acp executable: ', answer: realpathSync(process.env.BEEHIVE_REAL_BUZZ_ACP!) },
    { prompt: 'Buzz CONVERSATION relay URL', answer: `ws://127.0.0.1:${address.port}` },
    { prompt: 'Absolute installed buzz CLI executable: ', answer: realpathSync(join(process.env.BEEHIVE_REAL_BUZZ_ACP!, '..', 'buzz')) },
    ] : []),
    { prompt: 'Create a NEW agent identity', answer: 'yes' },
    { prompt: 'Add another independent NEW agent', answer: 'yes' },
    { prompt: 'Add another independent NEW agent', answer: 'no' },
  ]);
  if (oauth) { assert.match(setupOutput, /credentials UNCHECKED, readiness UNVERIFIED/); assert.match(setupOutput, /auth databricks_v2/); }
  const entry = installationSlots(installation)[0]!;
  const agent = entry.agent;
  if (!converting) assert.ok(entry.setup.conversation?.replyTool, 'normal wizard provisions conversation and tool without manual conversion');
  if (!converting) assert.match(setupOutput, /Normal conversation configured/);
  const sibling = installationSlots(installation)[1]!;
  const originalY = readFileSync(sibling.path);
  const originalA = structuredClone(entry.bindings.default);
  if (converting) await terminal(['local-setup', installation], [
    { prompt: 'Local action [', answer: buzz ? 'add-buzz-provider' : codex ? 'add-codex' : claude ? 'add-claude' : 'add-goose' },
    { prompt: 'Existing binding ID to reuse: ', answer: 'default' },
    { prompt: buzz ? 'NEW immutable Buzz Agent binding ID: ' : !goose ? `NEW immutable ${title} binding ID: ` : 'NEW immutable Goose binding ID: ', answer: 'B' },
    { prompt: buzz ? 'Absolute installed buzz-agent executable: ' : codex ? 'Absolute installed codex-acp adapter: ' : claude ? 'Absolute installed claude-agent-acp adapter: ' : 'Absolute installed Goose executable (runs acp): ', answer: runner },
    ...(!goose && !buzz ? [
      { prompt: `Absolute installed ${kind} CLI: `, answer: realpathSync(process.execPath) },
      { prompt: `Absolute owner-only local ${codex ? 'OPENAI' : 'ANTHROPIC'}_API_KEY file`, answer: keyFile },
      { prompt: `Operator-approved compatible exact ${title} model IDs`, answer: model },
    ] : []),
    ...(buzz ? [
      { prompt: 'Buzz Agent provider [', answer: provider },
      { prompt: `Absolute owner-only local ${keyName} file`, answer: keyFile },
      { prompt: 'Provider HTTPS base URL', answer: endpoint },
      { prompt: 'Operator-approved compatible exact Buzz Agent model IDs', answer: model },
    ] : []),
    { prompt: 'Allowed workspace (absolute directory): ', answer: dir },
    { prompt: buzz ? 'Existing dedicated Buzz Agent service HOME' : !goose ? `Existing dedicated ${title} service HOME` : 'Existing dedicated service HOME', answer: entry.setup.serviceHome! },
    ...(goose ? [{ prompt: 'Locally configured Goose provider ID: ', answer: 'fixture-provider' },
    { prompt: 'Operator-approved compatible exact model IDs', answer: 'goose-model-a' }] : []),
    ...(codex ? [{ prompt: 'Existing dedicated CODEX_HOME', answer: entry.setup.configDirectory! }] : []),
    ...(buzz ? [{ prompt: 'Existing dedicated Buzz Agent config directory', answer: entry.setup.configDirectory! }] : []),
    { prompt: buzz ? 'Save NEW Buzz Agent binding only' : !goose ? `Save NEW ${title} binding only` : 'Save NEW Goose binding only', answer: 'yes' },
  ]);
  assert.ok(!setupOutput.includes(ownerSecret));
  if (custom) { assert.ok(!setupOutput.includes('owner-only-custom-value')); assert.ok(!setupOutput.includes(literal)); }

  // Lifecycle decisions come from signed public inventory, not a private journal.
  let inventory: Message | undefined;
  const state = () => {
    assert.ok(inventory, 'public host inventory required');
    return { revision: inventory.revision, phase: inventory.body.phase,
      selected: inventory.body.selectedNext as any, actual: inventory.body.actualRun as any };
  };
  const channel = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee';
  const replies: any[] = [];
  const subscriptionsBySocket = new Map<string, { socket: any; id: string }>();
  let expected: any;
  let channelReads = 0;
  let authenticated = false; let subscriptions = 0; let delivered = false;
  function observeReply(e: any) {
    if (![9, 40002].includes(e.kind) || e.pubkey !== agent) return;
    const hash = createHash('sha256').update(JSON.stringify([0, e.pubkey, e.created_at, e.kind, e.tags, e.content])).digest('hex');
    assert.equal(e.id, hash); assert.ok(schnorr.verify(e.sig, hash, agent));
    assert.deepEqual(e.tags.filter((t: string[]) => t[0] === 'p').map((t: string[]) => t[1]), [recipient]);
    assert.ok(e.tags.some((t: string[]) => t[0] === 'h' && t[1] === expected.tags.find((t: string[]) => t[0] === 'h')[1]));
    assert.ok(e.tags.some((t: string[]) => t[0] === 'e' && t[1] === expected.id));
    assert.ok(e.content.includes('Private conversation fixture response'));
    replies.push(e); delivered = true;
  }
  function event(kind: number, tags: string[][], content: string, key = ownerSecret) {
    const pubkey = publicKey(key); const created_at = Math.floor(Date.now() / 1000);
    const id = createHash('sha256').update(JSON.stringify([0, pubkey, created_at, kind, tags, content])).digest('hex');
    return { id, pubkey, created_at, kind, tags, content, sig: Buffer.from(schnorr.sign(id, key)).toString('hex') };
  }
  const recipientKey = newKey(); const recipient = publicKey(recipientKey);
  writeFileSync(join(dir, 'recipient'), recipient);
  const outsiderKey = newKey();
  writeFileSync(join(dir, 'outsider'), publicKey(outsiderKey));
  const secondChannel = 'bbbbbbbb-cccc-4ddd-8eee-ffffffffffff';
  const members = event(39002, [['d', channel], ['p', agent], ['p', owner], ['p', recipient]], '');
  const metadata = event(39000, [['d', channel], ['name', 'fixture'], ['type', 'channel']], '');
  const inbound = event(9, [['h', channel], ['p', agent]], 'Please respond to this isolated fixture conversation.');
  const secondMembers = event(39002, [['d', secondChannel], ['p', agent], ['p', owner], ['p', recipient]], '');
  const secondMetadata = event(39000, [['d', secondChannel], ['name', 'second'], ['type', 'channel']], '');
  const second = event(9, [['h', secondChannel], ['p', agent]], 'Second isolated owner conversation.');
  const later = event(9, [['h', channel], ['p', agent], ['e', inbound.id, '', 'root']], 'Later prompt in first thread.');
  const inbounds = [inbound, second, later]; expected = inbound;
  http.on('request', async (req, res) => {
    let body = ''; for await (const chunk of req) body += chunk;
    res.setHeader('Content-Type', 'application/json');
    if (req.url?.includes('/query')) {
      const filters = JSON.parse(body); const serialized = JSON.stringify(filters);
      if (serialized.includes('39002')) channelReads++;
      res.end(JSON.stringify(serialized.includes('39002') ? [members, secondMembers] : serialized.includes('39000') ? [metadata, secondMetadata] : inbounds.filter(e => serialized.includes(e.id))));
    } else if (req.url?.includes('/events')) {
      const e = JSON.parse(body); observeReply(e);
      res.end(JSON.stringify({ id: e.id, accepted: true }));
    } else res.end('[]');
  });
  const ws = new WebSocketServer({ server: http });
  ws.on('connection', socket => {
    socket.send(JSON.stringify(['AUTH', 'beehive-isolated-challenge']));
    socket.on('message', bytes => {
      const m = JSON.parse(bytes.toString());
      if (m[0] === 'AUTH') {
        const e = m[1];
        assert.equal(e.pubkey, agent); assert.equal(e.kind, 22242);
        assert.ok(e.tags.some((t: string[]) => t[0] === 'challenge' && t[1] === 'beehive-isolated-challenge'));
        const hash = createHash('sha256').update(JSON.stringify([0, e.pubkey, e.created_at, e.kind, e.tags, e.content])).digest('hex');
        assert.equal(e.id, hash); assert.ok(schnorr.verify(e.sig, hash, e.pubkey));
        authenticated = true; socket.send(JSON.stringify(['OK', e.id, true, '']));
      } else if (m[0] === 'EVENT') { observeReply(m[1]); socket.send(JSON.stringify(['OK', m[1].id, true, ''])); }
      else if (m[0] === 'REQ') {
        subscriptions++; socket.send(JSON.stringify(['EOSE', m[1]]));
        for (const f of m.slice(2)) for (const c of f['#h'] ?? []) { subscriptionsBySocket.set(c, { socket, id: m[1] }); }
        if (m.slice(2).some((f: any) => f['#h']?.includes(channel))) setTimeout(() => socket.send(JSON.stringify(['EVENT', m[1], inbound])), 100);
      }
    });
  });
  assert.equal(installationSlots(installation)[0]!.agent, agent);
  const management = await relay(0, owner, join(dir, 'management.json'));
  const ma = management.address(); assert.ok(ma && typeof ma !== 'string');
  const managementURL = `ws://127.0.0.1:${ma.port}`;
  const observer = connect(managementURL, ownerSecret, m => {
    if (m.type === 'inventory' && m.host === 'journey' && m.agent === agent) inventory = m;
  });
  await observer.ready;
  const service = async () => {
    const child = spawn(process.execPath, ['src/cli.ts', 'host', installation, managementURL], { stdio: ['ignore', 'pipe', 'pipe'] });
    let output = '', errors = '';
    child.stdout.on('data', b => output = (output + b).slice(-8192)); child.stderr.on('data', b => errors = (errors + b).slice(-8192));
    const exited = once(child, 'exit');
    let closing: Promise<void> | undefined;
    const close = () => closing ??= (async () => {
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
      assert.deepEqual(await exited, [0, null], errors);
      assert.equal(errors, '');
      assert.equal(existsSync(join(installation, 'host.lock')), false);
    })();
    try {
      for (let n = 0; n < 400 && !(output.includes('Host online;') && inventory); n++) {
        assert.equal(child.exitCode, null, errors); await delay(20);
      }
      assert.match(output, /Host online;/); assert.ok(inventory);
      return { close };
    } catch (error) { await close(); throw error; }
  };
  let h = await service();
  if (converting) {
    await terminal(['tui', identity, managementURL], [
      { prompt: 'beehive> ', answer: 'select 1' },
      { prompt: 'beehive> ', answer: 'start' },
      { prompt: 'beehive> ', answer: 'binding B', gate: () => state().phase === 'running', observedRevision: 1 },
      { prompt: 'beehive> ', answer: 'stop', gate: () => state().selected.harnessSetup?.id === 'B', observedRevision: 2 },
      { prompt: 'beehive> ', answer: 'quit', gate: () => state().phase === 'stopped', observedRevision: 3 },
    ]);
    await h.close();
    const beforeJournal = readFileSync(entry.path);
    const beforeB = installationSlots(installation)[0]!.bindings.B;
    const output = await terminal(['local-setup', installation], [
      { prompt: 'Local action [', answer: 'normal' },
      { prompt: 'Existing agent public key: ', answer: agent },
      { prompt: 'NEW immutable normal binding ID: ', answer: 'B-normal' },
      { prompt: 'Absolute installed buzz-acp executable: ', answer: realpathSync(process.env.BEEHIVE_REAL_BUZZ_ACP!) },
      { prompt: 'Buzz CONVERSATION relay URL', answer: `ws://127.0.0.1:${address.port}` },
      { prompt: 'Absolute installed buzz CLI executable: ', answer: realpathSync(join(process.env.BEEHIVE_REAL_BUZZ_ACP!, '..', 'buzz')) },
      { prompt: 'Save NEW normal binding under installation lock', answer: 'yes' },
    ]);
    assert.match(output, /saved from selected B/);
    assert.deepEqual(readFileSync(entry.path), beforeJournal);
    assert.deepEqual(installationSlots(installation)[0]!.bindings.B, beforeB);
    assert.deepEqual(installationSlots(installation)[0]!.bindings.default, originalA);
    const normal = installationSlots(installation)[0]!.bindings['B-normal']!;
    const replaced = await terminal(['local-setup', installation], [
      { prompt: 'Local action [', answer: 'replace-binding' },
      { prompt: 'Existing binding ID to reuse: ', answer: 'B-normal' },
      { prompt: 'NEW immutable binding ID: ', answer: 'B-replacement' },
      { prompt: 'Absolute compatible executable: ', answer: normal.runner },
      { prompt: 'Allowed workspace (absolute directory): ', answer: normal.workspace },
      { prompt: 'Retire B-normal and replace with B-replacement, without selecting it? [yes/no]: ', answer: 'yes' },
      { prompt: 'Save NEW binding only (no selection, key change or restart)? [yes/no]: ', answer: 'yes' },
    ]);
    assert.match(replaced, /Binding B-replacement saved/);
    assert.deepEqual(readFileSync(entry.path), beforeJournal);
    const afterReplacement = installationSlots(installation)[0]!;
    assert.deepEqual(afterReplacement.bindings['B-normal'], normal);
    assert.deepEqual(afterReplacement.bindings['B-replacement'], normal);
    assert.ok(afterReplacement.retiredBindings?.['B-normal']);
    inventory = undefined; h = await service();
    await terminal(['tui', identity, managementURL], [
      { prompt: 'beehive> ', answer: 'select 1' },
      { prompt: 'beehive> ', answer: 'binding B-replacement', observedRevision: 3 },
      { prompt: 'beehive> ', answer: 'quit', gate: () => state().selected.harnessSetup?.id === 'B-replacement', observedRevision: 4 },
    ]);
  }
  const seen: Message[] = [];
  const controller = connect(managementURL, ownerSecret, m => seen.push(m)); await controller.ready;
  const instructions = 'NORMAL-PROFILE-BOUNDARY: preserve the exact selected behavior.';
  const behavior = { name: 'Normal profile', parent: null, instructions, revision: profileRevision('Normal profile', null, instructions) };
  controller.send(message('profile', 'profiles', 'profiles', 0, behavior));
  const save = message('save', 'journey', agent, state().revision, { ...state().selected, profile: behavior.revision, behavior });
  controller.send(save);
  for (let n = 0; n < 400 && !seen.some(m => m.type === 'receipt' && m.body.operation === save.id); n++) await delay(20);
  assert.match(String(seen.find(m => m.type === 'receipt' && m.body.operation === save.id)?.body.result), /saved/);
  controller.close();
  const convertedManifest = readFileSync(join(installation, 'setup.json'));

  const session = { ready: Promise.resolve(),
    verify: async () => state().actual.evidence,
    get healthy() { return state().phase === 'running'; },
    stop: async () => { await h.close(); for (const c of management.clients) c.terminate(); await new Promise<void>(r => management.close(() => r())); },
  };
  try {
    await terminal(['tui', identity, managementURL], [
      { prompt: 'beehive> ', answer: 'select 1', gate: () => true },
      { prompt: 'beehive> ', answer: 'start' },
      { prompt: 'beehive> ', answer: 'show', gate: () => state().phase === 'running' },
      { prompt: 'beehive> ', answer: 'quit' },
    ]);
    for (let i = 0; i < 100 && !subscriptions; i++) await delay(50);
    assert.ok(authenticated, 'installed executable must authenticate with the provisioned key');
    assert.ok(subscriptions > 0, 'installed executable must enter subscription loop');
    const evidence = await session.verify();
    if (custom) {
      for (const privateValue of ['owner-only-custom-value', literal, customFile, entry.setup.serviceHome!]) assert.ok(!JSON.stringify(inventory).includes(privateValue), 'custom local input never advertised');
      assert.ok(!existsSync(join(dir, 'shell-was-invoked')), 'structured argv never invokes shell');
    }
    if (buzz) {
      assert.ok(existsSync(join(dir, `${kind}-env-checked`)), 'distinct Buzz provider env contract');
      for (const value of [`fixture-${kind}-private-key`, keyFile, entry.setup.serviceHome!, endpoint]) assert.ok(!JSON.stringify(inventory).includes(value), 'Buzz provider private input never advertised');
      assert.ok(!setupOutput.includes(`fixture-${kind}-private-key`));
    }
    if (!goose && !buzz) {
      assert.ok(existsSync(join(dir, `${kind}-env-checked`)));
      assert.ok(!readFileSync(join(dir, `${kind}-rpc-methods`), 'utf8').includes('session/set_model'));
      for (const privateValue of [`fixture-${buzz ? kind : codex ? 'codex' : 'claude'}-private-key`, keyFile, entry.setup.serviceHome!]) assert.ok(!JSON.stringify(inventory).includes(privateValue), 'private Claude context never advertised');
      assert.ok(!setupOutput.includes(`fixture-${buzz ? kind : codex ? 'codex' : 'claude'}-private-key`));
    }
    assert.equal(state().actual.evidence.source, 'external-buzz-conversation');
    assert.match(readFileSync(join(dir, 'received-system-instructions'), 'utf8'), /NORMAL-PROFILE-BOUNDARY/);
    assert.equal(state().actual.selection.behavior.revision, behavior.revision);
    if (goose) assert.match(evidence.session, /^goose-[0-9]+-1$/); else assert.match(evidence.session, new RegExp(`^${kind}-[0-9]+-1$`));
    assert.equal(evidence.agentPublicKey, agent);
    assert.equal(evidence.model, model);
    assert.ok(delivered, 'the actual Buzz CLI must publish a signed threaded agent reply, not just return tool JSON');
    const unauthorized = event(9, [['h', channel], ['p', agent]], 'Not the configured owner.', recipientKey);
    const firstSubscription = subscriptionsBySocket.get(channel)!;
    firstSubscription.socket.send(JSON.stringify(['EVENT', firstSubscription.id, unauthorized]));
    await delay(1200); assert.equal(replies.length, 1, 'upstream owner-only admission rejects a different sender');
    for (const next of [second, later]) {
      const previous: number = replies.length; expected = next;
      const c = next.tags.find(t => t[0] === 'h')![1]!;
      const subscription = subscriptionsBySocket.get(c); assert.ok(subscription);
      subscription.socket.send(JSON.stringify(['EVENT', subscription.id, next]));
      for (let i = 0; i < 300 && replies.length === previous; i++) await delay(50);
      assert.equal(replies.length, previous + 1, 'each later conversation must produce its own signed reply');
    }
    for (let i = 0; i < 100 && readFileSync(join(dir, 'completed-tools'), 'utf8').trim().split('\n').length < 3; i++) await delay(50);
    assert.equal(readFileSync(join(dir, 'rejected-mentions'), 'utf8').trim().split('\n').length, 3);
    assert.equal(replies.length, 3); assert.equal(new Set(replies.map(e => e.id)).size, 3);
    assert.ok(channelReads >= 3, 'additional CLI channels list operation exercised');
    assert.equal(readFileSync(join(dir, 'harness-identity'), 'utf8'), agent);
    await delay(100); assert.ok(session.healthy, 'broker remains model-confirmed and healthy after later turns');
    const oldActual = state().actual;
    const manifest = readFileSync(join(installation, 'setup.json'));
    const restartInstructions = 'NORMAL-PROFILE-BOUNDARY: freshly changed Restart behavior.';
    const restartBehavior = { name: 'Normal profile', parent: behavior.revision, instructions: restartInstructions, revision: profileRevision('Normal profile', behavior.revision, restartInstructions) };
    const profileClient = connect(managementURL, ownerSecret, m => seen.push(m)); await profileClient.ready;
    profileClient.send(message('profile', 'profiles', 'profiles', 0, restartBehavior));
    const changed = message('save', 'journey', agent, state().revision, { ...state().selected, profile: restartBehavior.revision, behavior: restartBehavior });
    profileClient.send(changed);
    for (let n = 0; n < 400 && !seen.some(m => m.type === 'receipt' && m.body.operation === changed.id); n++) await delay(20);
    assert.match(String(seen.find(m => m.type === 'receipt' && m.body.operation === changed.id)?.body.result), /saved/);
    profileClient.close();
    assert.deepEqual(state().actual, oldActual);
    expected = inbound;
    await terminal(['tui', identity, managementURL], [
      { prompt: 'beehive> ', answer: 'select 1' },
      { prompt: 'beehive> ', answer: 'restart', observedRevision: state().revision },
      { prompt: 'beehive> ', answer: 'show', gate: () => state().phase === 'running' && state().actual.run !== oldActual.run },
      { prompt: 'beehive> ', answer: 'quit' },
    ]);
    assert.notEqual(state().actual.evidence.session, oldActual.evidence.session);
    assert.equal(state().actual.evidence.model, oldActual.evidence.model);
    assert.deepEqual((readPrivate(entry.path) as any).runs[oldActual.run], oldActual, 'fresh fixture history audit only; never used to admit execution');
    assert.deepEqual(readFileSync(join(installation, 'setup.json')), manifest);
    assert.deepEqual(readFileSync(sibling.path), originalY);
    assert.deepEqual(readFileSync(join(installation, 'setup.json')), convertedManifest);
    assert.equal(state().actual.selection.behavior.revision, restartBehavior.revision);
    assert.equal(replies.length, 4, 'Restart must produce a fresh installed signed reply');
    assert.match(readFileSync(join(dir, 'received-system-instructions'), 'utf8'), /NORMAL-PROFILE-BOUNDARY/);
    if (buzz) {
      assert.equal(readFileSync(join(dir, 'received-system-instructions'), 'utf8'), restartInstructions);
      const inputs = readFileSync(join(dir, 'native-session-inputs.jsonl'), 'utf8').trim().split('\n').map(line => JSON.parse(line));
      assert.ok(inputs.some(row => row.systemPrompt === instructions));
      assert.ok(inputs.filter(row => row.systemPrompt === restartInstructions).length >= 2, 'fresh preflight and actual native session inputs');
      assert.ok(new Set(inputs.filter(row => row.systemPrompt === restartInstructions).map(row => row.pid)).size >= 2);
    }
    const controlSeen: Message[] = [];
    const control = connect(managementURL, ownerSecret, m => controlSeen.push(m)); await control.ready;
    const request = async (m: Message) => {
      control.send(m);
      for (let n = 0; n < 400; n++) {
        const receipt = controlSeen.find(e => e.type === 'receipt' && e.body.operation === m.id);
        if (receipt) return receipt;
        await delay(20);
      }
      throw Error('Missing control receipt');
    };
    try {
      const actual = state().actual;
      writeFileSync(join(dir, 'mode'), 'wrong-model');
      const rejected = message('restart', 'journey', agent, state().revision, {});
      assert.notEqual((await request(rejected)).body.result, 'accepted');
      assert.deepEqual(state().actual, actual, 'wrong model preflight preserves actual');
      if (!goose || custom) {
        for (const failure of ['missing-native', ...(!custom ? ['auth-rejected'] : ['missing-profile']), ...(oauth ? ['missing-refresh'] : []), ...(codex ? ['wrong-protocol', 'missing-model'] : []), ...(!goose && !claude && !codex && !custom ? ['wrong-protocol', 'missing-profile', 'current-drift', 'config-drift', 'coalesced-drift'] : [])]) {
          writeFileSync(join(dir, 'mode'), failure);
          assert.notEqual((await request(message('restart', 'journey', agent, state().revision, {}))).body.result, 'accepted');
          assert.deepEqual(state().actual, actual);
        }
      }
      if (buzz && !oauth) {
        writeFileSync(join(dir, 'mode'), 'ok');
        writeFileSync(keyFile, 'wrong-fixture-credential');
        assert.notEqual((await request(message('restart', 'journey', agent, state().revision, {}))).body.result, 'accepted');
        assert.deepEqual(state().actual, actual, 'wrong private credential preserves actual');
      }
      if (!goose && !oauth) {
        rmSync(keyFile);
        const failedAuth = await request(message('restart', 'journey', agent, state().revision, {}));
        assert.match(String(failedAuth.body.result), new RegExp(`${buzz ? 'Buzz Agent' : title} API key prerequisite`));
        assert.deepEqual(state().actual, actual);
      }
      writeFileSync(join(dir, 'mode'), 'ok');
      const stop = message('stop', 'journey', agent, state().revision, {});
      assert.equal((await request(stop)).body.result, 'accepted');
      const stopped = readFileSync(entry.path);
      // Completed rejected Restart and Stop are replayed, not fresh execution authority.
      controlSeen.length = 0; await request(rejected);
      controlSeen.length = 0; await request(stop);
      assert.deepEqual(readFileSync(entry.path), stopped);
      assert.equal(state().phase, 'stopped'); assert.equal(state().actual, null);
      assert.deepEqual(readFileSync(sibling.path), originalY);
      assert.deepEqual(readFileSync(join(installation, 'setup.json')), convertedManifest);
      if (codex || custom) {
        await h.close();
        const methodsBefore = readFileSync(join(dir, custom ? 'goose-rpc-methods' : 'codex-rpc-methods'));
        const historyBefore = readFileSync(entry.path);
        await terminal(['remove-agent-key', installation, agent], [{ prompt: 'Remove this installation', answer: 'yes' }]);
        assert.deepEqual(readFileSync(entry.path), historyBefore);
        const keylessManifest = readFileSync(join(installation, 'setup.json'));
        assert.equal(installationSlots(installation).find(e => e.agent === agent)!.keyPresent, false);
        inventory = undefined; h = await service();
        for (const action of ['start', 'restart'] as const) {
          const receipt = await request(message(action, 'journey', agent, state().revision, {}));
          assert.match(String(receipt.body.result), /key removed/);
        }
        assert.equal((await request(message('stop', 'journey', agent, state().revision, {}))).body.result, 'accepted');
        assert.deepEqual(readFileSync(join(dir, custom ? 'goose-rpc-methods' : 'codex-rpc-methods')), methodsBefore, 'missing key cannot spawn another adapter');
        assert.deepEqual(readFileSync(join(installation, 'setup.json')), keylessManifest, 'no key recreation');
        assert.deepEqual(readFileSync(sibling.path), originalY);
      }

    } finally { control.close(); }
    console.log(`isolated installed runtime: NIP-42 verified, subscriptions=${subscriptions}, signed replies=${replies.map(e => e.id).join(',')}, agent=${agent}, two channels/three owner prompts; explicit non-owner member recipient=${recipient}`);

  } finally {
    if (goose && existsSync(join(dir, 'goose-rpc-methods'))) console.log(readFileSync(join(dir, 'goose-rpc-methods'), 'utf8'));
    observer.close();
    await session.stop();
    for (const file of ['harness-pid', 'descendant-pid', 'tool-shim-pid']) {
      if (!existsSync(join(dir, file))) continue;
      const pid = Number(readFileSync(join(dir, file), 'utf8'));
      assert.throws(() => process.kill(pid, 0), (e: any) => e.code === 'ESRCH', `${file} must be absent after owned Stop`);
    }
    for (const socket of ws.clients) socket.terminate();
    await new Promise<void>(resolve => ws.close(() => resolve()));
    await new Promise<void>(resolve => http.close(() => resolve()));
    rmSync(dir, { recursive: true, force: true });
  }
});
