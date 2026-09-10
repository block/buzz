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
for (const converting of [false, true]) test(`normal wizard + TUI Start/Restart + host CLI service subprocess + installed CLI signed replies (${converting ? 'selected diagnostic B conversion' : 'initial normal default'})`, { skip: !process.env.BEEHIVE_REAL_BUZZ_ACP }, async () => {
  const goose = true;
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-installed-')));
  writeFileSync(join(dir, 'mode'), 'ok');
  const ownerSecret = newKey(); const owner = publicKey(ownerSecret);
  const identity = join(dir, 'owner.json'), installation = join(dir, 'host');
  writePrivate(identity, { secret: ownerSecret });
  const runner = join(dir, 'goose');
  writeFileSync(runner, `#!${realpathSync(process.execPath)}\nif (process.argv[2] !== 'acp') throw Error('acp required');\nawait import(${JSON.stringify(pathToFileURL(resolve('test/conversation-harness-fixture.ts')).href)});\n`, { mode: 0o700 });
  const http = createServer();
  await new Promise<void>(resolve => http.listen(0, '127.0.0.1', resolve));
  const address = http.address(); assert.ok(address && typeof address !== 'string');
  const setupOutput = await terminal(['setup', installation, identity], [
    { prompt: 'Host name: ', answer: 'journey' }, { prompt: 'Setup [', answer: '3' },
    { prompt: 'Absolute installed Goose executable (runs acp): ', answer: runner },
    { prompt: 'Allowed workspace (absolute directory): ', answer: dir },
    { prompt: 'Additional allowed workspace (blank for none): ', answer: '' },
    { prompt: 'Locally configured Goose provider ID: ', answer: 'fixture-provider' },
    { prompt: 'Operator-approved compatible exact model IDs (comma-separated): ', answer: 'goose-model-a' },
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
  const entry = installationSlots(installation)[0]!;
  const agent = entry.agent;
  if (!converting) assert.ok(entry.setup.conversation?.replyTool, 'normal wizard provisions conversation and tool without manual conversion');
  if (!converting) assert.match(setupOutput, /Normal conversation configured/);
  const sibling = installationSlots(installation)[1]!;
  const originalY = readFileSync(sibling.path);
  const originalA = structuredClone(entry.bindings.default);
  if (converting) await terminal(['local-setup', installation], [
    { prompt: 'Local action [', answer: 'add-goose' },
    { prompt: 'Existing binding ID to reuse: ', answer: 'default' },
    { prompt: 'NEW immutable Goose binding ID: ', answer: 'B' },
    { prompt: 'Absolute installed Goose executable (runs acp): ', answer: runner },
    { prompt: 'Allowed workspace (absolute directory): ', answer: dir },
    { prompt: 'Existing dedicated service HOME', answer: entry.setup.serviceHome! },
    { prompt: 'Locally configured Goose provider ID: ', answer: 'fixture-provider' },
    { prompt: 'Operator-approved compatible exact model IDs', answer: 'goose-model-a' },
    { prompt: 'Save NEW Goose binding only', answer: 'yes' },
  ]);
  assert.ok(!setupOutput.includes(ownerSecret));
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
    inventory = undefined; h = await service();
    await terminal(['tui', identity, managementURL], [
      { prompt: 'beehive> ', answer: 'select 1' },
      { prompt: 'beehive> ', answer: 'binding B-normal', observedRevision: 3 },
      { prompt: 'beehive> ', answer: 'quit', gate: () => state().selected.harnessSetup?.id === 'B-normal', observedRevision: 4 },
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
    assert.equal(state().actual.evidence.source, 'external-buzz-conversation');
    assert.match(readFileSync(join(dir, 'received-system-instructions'), 'utf8'), /NORMAL-PROFILE-BOUNDARY/);
    assert.equal(state().actual.selection.behavior.revision, behavior.revision);
    if (goose) assert.match(evidence.session, /^goose-[0-9]+-1$/); else assert.equal(evidence.session, 'conversation-session');
    assert.equal(evidence.agentPublicKey, agent);
    assert.equal(evidence.model, goose ? 'goose-model-a' : 'databricks-claude-haiku-4-5');
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
    assert.equal(state().actual.selection.behavior.revision, behavior.revision);
    assert.equal(replies.length, 4, 'Restart must produce a fresh installed signed reply');
    assert.match(readFileSync(join(dir, 'received-system-instructions'), 'utf8'), /NORMAL-PROFILE-BOUNDARY/);
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
