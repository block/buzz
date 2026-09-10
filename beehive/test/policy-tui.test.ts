import { provisionSetup } from './provision.ts';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, realpathSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { WebSocket, WebSocketServer } from 'ws';
import { host } from '../src/host.ts';
import { relay } from '../src/relay.ts';
import { managementClient } from '../src/intents.ts';
import { connect } from '../src/client.ts';
import { message, newKey, publicKey, open } from '../src/protocol.ts';
import { writePrivate } from '../src/storage.ts';

async function until(predicate: () => boolean) {
  for (let n = 0; n < 240; n++) { if (predicate()) return; await delay(25); }
  throw Error('Missing real TUI recovery evidence');
}

test('actual TUI: policy repair, bounded unchanged retry, lost committed receipt reconciliation and Quit', async () => {
  for (const loss of ['before-publication', 'after-commit'] as const) {
    const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-policy-tui-')));
    const secret = newKey(); const agentSecret = newKey(); const agent = publicKey(agentSecret);
    writePrivate(join(dir, 'identity.json'), { secret });
    provisionSetup(join(dir, 'setup.json'), { host: 'policy-host', ownerSecret: secret, agentSecret,
      runner: process.execPath, args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' });
    const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
    const address = server.address(); assert.ok(address && typeof address !== 'string');
    const url = `ws://127.0.0.1:${address.port}`;
    const h = await host(dir, url);
    let relayVisibleRunning = false;
    const control = connect(url, secret, m => {
      // A private journal commit is not yet public relay inventory visibility.
      if (m.type === 'inventory' && m.host === 'policy-host' && m.agent === agent && m.revision === 1 && m.body.phase === 'running') relayVisibleRunning = true;
    }); await control.ready;
    const journal = () => JSON.parse(readFileSync(join(dir, 'journal.json'), 'utf8'));
    control.send(message('start', 'policy-host', agent, 0));
    await until(() => journal().phase === 'running');
    // Later TUI connections replay this durably committed revision before Stop.
    await until(() => relayVisibleRunning);
    let deny = true; let denials = 0;
    const attempts: string[] = [];
    const upstreams = new Set<WebSocket>();
    const proxy = new WebSocketServer({ host: '127.0.0.1', port: 0 });
    await new Promise<void>(resolve => proxy.once('listening', resolve));
    proxy.on('connection', downstream => {
      const upstream = new WebSocket(url); upstreams.add(upstream);
      const queued: string[] = [];
      upstream.on('open', () => { for (const data of queued) upstream.send(data); });
      downstream.on('message', raw => {
        const data = String(raw); const m = open(JSON.parse(data), secret);
        if (m.type === 'stop') {
          attempts.push(data);
          if (deny && loss === 'before-publication') { denials++; downstream.close(1008, 'Fixture policy'); return; }
        }
        if (upstream.readyState === WebSocket.OPEN) upstream.send(data); else queued.push(data);
      });
      upstream.on('message', raw => {
        const data = String(raw); const m = open(JSON.parse(data), secret);
        if (downstream.readyState === WebSocket.OPEN) downstream.send(data);
      });
      upstream.on('error', () => {}); downstream.on('error', () => {});
      downstream.on('close', () => { upstream.close(); upstreams.delete(upstream); });
    });
    if (loss === 'after-commit') {
      // Lose the committed Stop receipt BEFORE relay storage, not just at the UI.
      // Recovery must query the real host outbox; relay history is insufficient.
      const hostSocket = [...server.clients][0]!;
      const handlers = hostSocket.listeners('message'); hostSocket.removeAllListeners('message');
      hostSocket.on('message', (raw, binary) => {
        const m = open(JSON.parse(String(raw)), secret);
        if (deny && m.type === 'receipt' && attempts.length && m.body.operation === open(JSON.parse(attempts[0]!), secret).id) {
          assert.equal(journal().phase, 'stopped'); denials++;
          for (const socket of proxy.clients) socket.close(1008, 'Fixture committed receipt loss');
          return;
        }
        for (const handler of handlers) Reflect.apply(handler, hostSocket, [raw, binary]);
      });
    }
    const proxyAddress = proxy.address(); assert.ok(proxyAddress && typeof proxyAddress !== 'string');
    const proxyUrl = `ws://127.0.0.1:${proxyAddress.port}`;
    const children: ReturnType<typeof spawn>[] = [];
    function tui() {
      const child = spawn(process.execPath, ['src/cli.ts', 'tui', join(dir, 'identity.json'), proxyUrl], { stdio: ['pipe','pipe','pipe'] });
      children.push(child);
      let output = ''; child.stdout!.on('data', d => { output += String(d); }); child.stderr!.on('data', d => { output += String(d); });
      return {
        get output() { return output; },
        async ready() { await until(() => output.includes('beehive> ')); },
        async command(line: string, expected = 'beehive> ') {
          const offset = output.length; child.stdin!.write(`${line}\n`);
          await until(() => output.slice(offset).includes(expected));
        },
        async quit() {
          const offset = output.length; child.stdin!.write('quit\n');
          await until(() => child.exitCode !== null);
          assert.equal(child.exitCode, 0, output);
          assert.match(output.slice(offset), /UI closed; host lifetime is independent/);
          assert.doesNotMatch(output.slice(offset), /UNKNOWN|disconnected/);
        },
      };
    }
    try {
      let ui = tui(); await ui.ready();
      await ui.command('select policy-host');
      await ui.command('stop');
      await until(() => ui.output.includes('automatic retry disabled'));
      const original = attempts[0]; assert.ok(original);
      const operation = open(JSON.parse(original), secret);
      if (loss === 'before-publication') {
        assert.equal(journal().phase, 'running');
        await ui.command('reconcile'); await delay(150);
        await ui.command('retry 1', '[yes/no]: '); await ui.command('yes');
        await until(() => denials === 2); await delay(250);
        assert.equal(attempts.length, 2, 'repeated policy denial never loops');
        assert.equal(journal().phase, 'running');
      } else {
        assert.equal(journal().phase, 'stopped'); assert.equal(journal().revision, 2);
      }
      await ui.quit(); deny = false;
      const beforeReopen = attempts.length;
      ui = tui(); await ui.ready(); await delay(200);
      assert.equal(attempts.length, beforeReopen, 'reopen never republishes blocked intent');
      await ui.command('operations');
      if (loss === 'before-publication') {
        assert.match(ui.output, new RegExp(`1\\. policy-host stop \\| agent ${agent} \\| operation ${operation.id} at revision 1: unknown`));
        await ui.command('reconcile'); await delay(150);
        assert.equal(attempts.length, beforeReopen, 'query is not a retry');
        await ui.command('retry 1', '[yes/no]: '); await ui.command('yes');
      }
      await until(() => ui.output.includes(`policy-host stop | agent ${agent} | operation ${operation.id}: completed | accepted`));
      assert.equal(journal().revision, 2); assert.equal(journal().phase, 'stopped');
      assert.equal(Object.keys(journal().operations).length, 2, 'one Start and original Stop only');
      assert.equal(journal().operations[operation.id].reply.body.result, 'accepted');
      assert.ok(attempts.every(e => e === original), 'ID, signature, ciphertext, body and preconditions are byte-identical');
      const completedAttempts = attempts.length;
      await ui.command('retry 1');
      assert.match(ui.output, /Use operations to choose an unresolved policy-blocked operation/);
      assert.equal(attempts.length, completedAttempts);
      await ui.command('reconcile'); await delay(150);
      assert.equal(journal().revision, 2, 'reconcile cannot duplicate lifecycle effects');
      await ui.quit();
      console.log(`${loss}: exact-envelope attempts=${attempts.length}, denials=${denials}, revision=2, original Stop accepted; Quit clean`);
    } finally {
      for (const child of children) if (child.exitCode === null) { child.kill(); await new Promise<void>(resolve => child.once('exit', () => resolve())); }
      control.close(); await h.close();
      for (const s of proxy.clients) s.terminate(); for (const s of upstreams) s.terminate();
      await new Promise<void>(resolve => proxy.close(() => resolve()));
      for (const s of server.clients) s.terminate(); await new Promise<void>(resolve => server.close(() => resolve()));
      rmSync(dir, { recursive: true, force: true });
    }
  }
});

test('actual reopened TUI identifies both same-host/action/revision blocked agents in rows and retry confirmations', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-two-agent-recovery-')));
  const secret = newKey(), agents = [publicKey(newKey()), publicKey(newKey())];
  writePrivate(join(dir, 'identity.json'), { secret });
  const server = new WebSocketServer({ host: '127.0.0.1', port: 0 });
  await new Promise<void>(resolve => server.once('listening', resolve));
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`;
  let attempts = 0;
  server.on('connection', socket => socket.on('message', raw => {
    if (open(JSON.parse(String(raw)), secret).type === 'stop') { attempts++; socket.close(1008, 'Fixture policy'); }
  }));
  const client = managementClient(join(dir, 'management-intents'), url, secret, () => {});
  let child: ReturnType<typeof spawn> | undefined;
  try {
    await client.ready;
    const requests = agents.map(agent => message('stop', 'shared', agent, 0));
    for (const request of requests) client.submit(request);
    await until(() => client.status().every(o => o.retryAvailable)); client.close();
    const before = attempts;
    child = spawn(process.execPath, ['src/cli.ts', 'tui', join(dir, 'identity.json'), url], { stdio: ['pipe','pipe','pipe'] });
    let output = ''; child.stdout!.on('data', d => { output += String(d); }); child.stderr!.on('data', d => { output += String(d); });
    await until(() => output.includes('beehive> '));
    async function command(line: string, prompt = 'beehive> ') {
      const offset = output.length; child!.stdin!.write(`${line}\n`);
      await until(() => output.slice(offset).includes(prompt)); return output.slice(offset);
    }
    const rows = await command('operations');
    for (const request of requests) {
      const label = `shared stop | agent ${request.agent} | operation ${request.id}`;
      const row = rows.split('\n').find(line => line.includes(label)); assert.ok(row, rows);
      assert.ok(row.includes('at revision 0: unknown'));
      const number = row.match(/^(\d+)\./)?.[1]; assert.ok(number);
      const warning = await command(`retry ${number}`, '[yes/no]: ');
      assert.ok(warning.includes(`Retry original ${label} at revision 0 ONCE`), warning);
      await command('no');
    }
    child.stdin!.write('quit\n'); await until(() => child!.exitCode !== null);
    assert.equal(child.exitCode, 0, output); assert.equal(attempts, before, 'declined retry never publishes either agent');
  } finally {
    client.close();
    if (child && child.exitCode === null) { child.kill(); await new Promise<void>(resolve => child!.once('exit', () => resolve())); }
    for (const socket of server.clients) socket.terminate();
    await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
  }
});
