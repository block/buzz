import { trace, beginTrace, endTrace } from '../src/latency-trace.ts';
import { provisionSetup } from './provision.ts';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, rmSync, readFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { host } from '../src/host.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';
import { message, newKey, publicKey, type Message, open } from '../src/protocol.ts';
import { writePrivate } from '../src/storage.ts';

async function until(predicate: () => boolean) {
  for (let i = 0; i < 300; i++) { if (predicate()) { trace('test.until.satisfied'); return; } await delay(25); trace('test.until.wake'); }
  throw Error('Missing Start/Stop cancellation evidence');
}

/** A Stop published beside a still-pending Start must cancel that admission
 * before any run commits, whether the pair arrives live in one batch or through
 * disconnected relay-history replay. No accepted live run may remain. */
test('a Stop received beside a pending Start cancels it before running, live and across replay', async () => {
  for (const scenario of ['replay', 'live', 'fixture-live'] as const) {
    const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-admission-')));
    const secret = newKey(); const agentSecret = newKey(); const agent = publicKey(agentSecret);
    const acp = scenario !== 'fixture-live';
    provisionSetup(join(dir, 'setup.json'), { host: 'admission-host', ownerSecret: secret, agentSecret,
      runner: process.execPath, args: acp ? [resolve('test/acp-fixture.ts'), 'delayed'] : [resolve('test/runner.ts')],
      workspace: dir, mode: acp ? 'buzz-agent-databricks-v2' : 'fixture',
      ...(acp ? { serviceHome: dir, configDirectory: dir, databricksHost: 'https://fixture.invalid' } : {}) });
    const storagePhases: { phase: string; at: number }[] = [];
    const server = await relay(0, publicKey(secret), join(dir, 'relay.json'), async phase => {
      storagePhases.push({ phase, at: Date.now() });
      if (storagePhases.length > 64) storagePhases.shift();
    });
    const address = server.address(); assert.ok(address && typeof address !== 'string');
    const url = `ws://127.0.0.1:${address.port}`;
    const h = await host(dir, url);
    const hostSocket = [...server.clients][0]; assert.ok(hostSocket);
    const seen: Message[] = [];
    const ui = connect(url, secret, m => seen.push(m)); await ui.ready;
    const journal = () => JSON.parse(readFileSync(join(dir, 'journal.json'), 'utf8'));
    try {
      if (scenario === 'replay') hostSocket.terminate(); // Publish while disconnected; recovery replays relay history.
      const began = Date.now(); trace('test.timer.begin', { began });
      const start = message('start', 'admission-host', agent, 0);
      const stop = message('stop', 'admission-host', agent, 0);
      ui.send(start); ui.send(stop); // Same batch as the pending Start: admission must not escape its preceding Stop.
      await until(() => seen.some(m => m.type === 'receipt' && m.body.operation === stop.id));
      assert.equal(seen.find(m => m.type === 'receipt' && m.body.operation === stop.id)?.body.result, 'accepted');
      assert.notEqual(journal().operations[start.id].reply.body.result, 'accepted');
      assert.match(String(journal().operations[start.id].reply.body.result), /cancelled/);
      assert.equal(journal().phase, 'stopped'); assert.equal(journal().actual, null); assert.equal(journal().revision, 1);
      assert.ok(Date.now() - began < 2500, `Cancellation must not wait for the four-second fixture prompt: ${JSON.stringify({ scenario, elapsed: Date.now() - began, storagePhases, receipts: seen.filter(m => m.type === 'receipt') })}`);
      assert.ok(!seen.some(m => m.type === 'inventory' && m.body.phase === 'running'), 'No live run may be left behind');
      await h.close(); await delay(200);
      assert.ok(!existsSync(join(dir, 'host.lock')), 'Verified stopped state must release the ownership lock');
      assert.equal(server.clients.size, 1, 'Closed host must not reconnect');
    } finally {
      await h.close(); ui.close(); for (const socket of server.clients) socket.terminate();
      await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true }); endTrace();
    }
  }
});

/** The same-batch cancellation fence stays exact: a wrong-authority Stop cannot
 * cancel, a Stop published before its Start cannot cancel that later Start, and
 * replayed already-terminal operations or a stale revision cannot cancel or rerun. */
test('same-batch Stop cancellation keeps authority, ordering, duplicate and revision fences', async t => {
  t.after(endTrace); // Retain timing even if owned cleanup itself fails.
  for (const control of ['wrong-authority', 'conflicting-id', 'invalid-body', 'stop-first', 'duplicate-replay'] as const) {
    beginTrace(control); const dir = realpathSync(mkdtempSync(join(tmpdir(), `beehive-admission-${control}-`)));
    const secret = newKey(); const agentSecret = newKey(); const agent = publicKey(agentSecret);
    provisionSetup(join(dir, 'setup.json'), { host: 'admission-host', ownerSecret: secret, agentSecret,
      runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' });
    const storagePhases: { phase: string; at: number }[] = [];
    const server = await relay(0, publicKey(secret), join(dir, 'relay.json'), async phase => {
      storagePhases.push({ phase, at: Date.now() });
      if (storagePhases.length > 64) storagePhases.shift();
    });
    const address = server.address(); assert.ok(address && typeof address !== 'string');
    const url = `ws://127.0.0.1:${address.port}`;
    const h = await host(dir, url);
    const hostSocket = [...server.clients][0]; assert.ok(hostSocket);
    const seen: Message[] = [];
    const ui = connect(url, secret, m => seen.push(m)); await ui.ready;
    const uiSocket = [...server.clients].find(s => s !== hostSocket); assert.ok(uiSocket);
    const journal = () => JSON.parse(readFileSync(join(dir, 'journal.json'), 'utf8'));
    const receipt = (id: string) => seen.find(m => m.type === 'receipt' && m.body.operation === id)?.body.result;
    try {
      hostSocket.terminate(); // All controls replay through disconnected relay history, like the defect.
      const began = Date.now(); trace('test.timer.begin', { began });
      if (['wrong-authority','conflicting-id','invalid-body'].includes(control)) {
        const start = message('start', 'admission-host', agent, 0);
        const wrongStop = message('stop', 'admission-host', publicKey(newKey()), 0);
        if (control === 'conflicting-id') { wrongStop.agent = agent; wrongStop.id = start.id; }
        if (control === 'invalid-body') { wrongStop.agent = agent; wrongStop.body = { invalid: true }; }
        ui.send(start); ui.send(wrongStop);
        await until(() => receipt(start.id) !== undefined && receipt(wrongStop.id) !== undefined);
        assert.equal(receipt(start.id), 'accepted');
        if (control === 'wrong-authority') assert.equal(receipt(wrongStop.id), 'not-authority');
        if (control === 'conflicting-id') await until(() => seen.some(m => m.body.operation === start.id && m.body.result === 'operation-id-conflict'));
        assert.equal(journal().phase, 'running'); assert.equal(journal().revision, 1);
        const stop = message('stop', 'admission-host', agent, 1); ui.send(stop);
        await until(() => receipt(stop.id) !== undefined);
        assert.equal(receipt(stop.id), 'accepted');
        assert.equal(journal().phase, 'stopped'); assert.equal(journal().actual, null); assert.equal(journal().revision, 2);
      } else if (control === 'stop-first') {
        const stop = message('stop', 'admission-host', agent, 0);
        const start = message('start', 'admission-host', agent, 0);
        ui.send(stop); ui.send(start); // A Stop cannot cancel a Start it never saw.
        await until(() => receipt(start.id) !== undefined && receipt(stop.id) !== undefined);
        assert.equal(receipt(stop.id), 'accepted'); assert.equal(receipt(start.id), 'revision-conflict');
        assert.equal(journal().phase, 'stopped'); assert.equal(journal().actual, null); assert.equal(journal().revision, 1);
      } else {
        const start = message('start', 'admission-host', agent, 0);
        const stop = message('stop', 'admission-host', agent, 0);
        ui.send(start); ui.send(stop);
        await until(() => receipt(stop.id) !== undefined);
        assert.equal(receipt(stop.id), 'accepted'); assert.equal(journal().phase, 'stopped');
        const receipts = journal().operations;
        // Replay the same committed batch after another disconnect: both operations
        // are already terminal, so history must not rerun the Start or recancel anything.
        const replayed = [...server.clients].find(s => s !== uiSocket); assert.ok(replayed);
        replayed.terminate();
        await until(() => [...server.clients].some(s => s !== uiSocket && s !== replayed));
        await delay(300);
        assert.deepEqual(journal().operations, receipts);
        assert.equal(journal().phase, 'stopped'); assert.equal(journal().actual, null); assert.equal(journal().revision, 1);
        const stale = message('stop', 'admission-host', agent, 0); ui.send(stale);
        await until(() => receipt(stale.id) !== undefined);
        assert.equal(receipt(stale.id), 'revision-conflict');
        assert.equal(journal().phase, 'stopped'); assert.equal(journal().revision, 1);
      }
      trace('test.timer.end', { elapsed: Date.now() - began }); assert.ok(Date.now() - began < 2500, `Controls must resolve without waiting on a run: ${JSON.stringify({ control, elapsed: Date.now() - began, storagePhases, receipts: seen.filter(m => m.type === 'receipt'), order: JSON.parse(readFileSync(join(dir, 'relay.json'), 'utf8')).map((e: unknown) => { const m = open(e, secret); return { type: m.type, id: m.id, revision: m.revision }; }) })}`);
      if (!['wrong-authority','conflicting-id','invalid-body'].includes(control)) assert.ok(!seen.some(m => m.type === 'inventory' && m.body.phase === 'running'), 'No live run may be left behind');
    } finally {
      await h.close(); ui.close(); for (const socket of server.clients) socket.terminate();
      await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true }); endTrace();
    }
  }
});
