import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, rmSync, readFileSync, existsSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { host } from '../src/host.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';
import { message, newKey, publicKey, type Message } from '../src/protocol.ts';
import { writePrivate } from '../src/storage.ts';

test('in-flight ACP Start is cancellable by authorized Stop and host close; malformed completion cannot commit running', async () => {
  for (const target of ['probe', 'conversation']) for (const action of ['stop', 'close', 'bad-tail']) {
    const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-cancel-')));
    const secret = newKey(); const agentSecret = newKey(); const agent = publicKey(agentSecret);
    const runtime = join(dir, 'runtime');
    writeFileSync(runtime, `#!/bin/sh\nexec '${realpathSync(process.execPath)}' '${resolve('test/conversation-runtime-fixture.ts')}'\n`, { mode: 0o700 });
    writeFileSync(join(dir, 'mode'), action === 'bad-tail' ? 'bad-tail' : 'delayed');
    writePrivate(join(dir, 'setup.json'), { host: 'cancel-host', ownerSecret: secret, agentSecret, runner: realpathSync(process.execPath), args: target === 'probe' ? [resolve('test/acp-fixture.ts'), action === 'bad-tail' ? 'bad-tail' : 'delayed'] : [resolve('test/conversation-harness-fixture.ts')], ...(target === 'conversation' ? { conversation: { executable: runtime, relay: 'ws://127.0.0.1:1' } } : {}), workspace: dir, mode: 'buzz-agent-databricks-v2', serviceHome: dir, configDirectory: dir, databricksHost: 'https://fixture.invalid' });
    const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
    const address = server.address(); assert.ok(address && typeof address !== 'string');
    const url = `ws://127.0.0.1:${address.port}`;
    const h = await host(dir, url); const seen: Message[] = [];
    const client = connect(url, secret, m => seen.push(m)); await client.ready;
    async function until(predicate: () => boolean) {
      for (let i = 0; i < 160; i++) { if (predicate()) return; await delay(25); }
      throw Error('Missing cancellation evidence');
    }
    const start = message('start', 'cancel-host', agent, 0);
    const journal = () => JSON.parse(readFileSync(join(dir, 'journal.json'), 'utf8'));
    try {
      client.send(start);
      if (action !== 'bad-tail') {
        await until(() => existsSync(join(dir, 'prompt-started')));
        assert.equal(journal().phase, 'transitioning');
        if (action === 'stop') {
          // A different agent/revision/body must not cancel an admitted Start.
          client.send(message('stop', 'cancel-host', publicKey(newKey()), 0));
          client.send(message('stop', 'cancel-host', agent, 99));
          client.send(message('stop', 'cancel-host', agent, 0, { unexpected: true }));
          await delay(100); assert.equal(journal().phase, 'transitioning');
        }
        const began = Date.now();
        if (action === 'close') await h.close();
        else {
          const stop = message('stop', 'cancel-host', agent, 0); client.send(stop);
          await until(() => seen.some(m => m.type === 'receipt' && m.body.operation === stop.id));
          assert.equal(seen.find(m => m.type === 'receipt' && m.body.operation === stop.id)?.body.result, 'accepted');
        }
        assert.ok(Date.now() - began < 2500, 'Stop/close must interrupt the four-second prompt, not wait for it');
      } else await until(() => seen.some(m => m.type === 'receipt' && m.body.operation === start.id));
      assert.notEqual(journal().operations[start.id].reply.body.result, 'accepted');
      assert.equal(journal().phase, 'stopped'); assert.equal(journal().actual, null);
      assert.ok(!seen.some(m => m.type === 'inventory' && m.body.phase === 'running'));
    } finally {
      await h.close(); client.close(); for (const c of server.clients) c.terminate();
      await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
    }
  }
});
