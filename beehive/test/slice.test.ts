import { spawn } from 'node:child_process';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, realpathSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { newKey, publicKey, seal, open, message, parseMessage, type Message } from '../src/protocol.ts';
import { writePrivate } from '../src/storage.ts';
import { relay } from '../src/relay.ts';
import { host } from '../src/host.ts';
import { connect } from '../src/client.ts';

test('validated, encrypted, owner-authenticated envelopes',() => {
  const key = newKey(); const m = message('start','host','agent');
  const e = seal(m,key);
  assert.deepEqual(open(e,key),m);
  assert.throws(() => open(e,newKey()));
  assert.throws(() => open({...e,ciphertext:'00'},key));
  assert.throws(() => parseMessage({...m,revision:-1}));
  assert.throws(() => parseMessage({...m,secret:'bad'}));
});

test('real websocket relay, external runner, durable retry, UI close/reopen, host restart',async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(),'beehive-test-')));
  const secret = newKey(); const agentSecret = newKey(); const agent = publicKey(agentSecret);
  const hostDir = join(dir,'host'); mkdirSync(hostDir);
  writePrivate(join(hostDir,'setup.json'),{ host:'test-host',ownerSecret:secret,agentSecret,runner:process.execPath,args:[resolve('test/runner.ts')],workspace:dir,mode:'fixture' });
  const log = join(dir,'relay.json');
  const server = await relay(0,publicKey(secret),log);
  const address = server.address(); assert.ok(address && typeof address !== 'string');
  const url = `ws://127.0.0.1:${address.port}`;
  await assert.rejects(host(hostDir, 'http://127.0.0.1:1'), /Development transport/);
  const unused = await relay(0,publicKey(secret),join(dir,'unused.json'));
  const unavailable = unused.address(); assert.ok(unavailable && typeof unavailable !== 'string');
  await new Promise<void>(resolve => unused.close(() => resolve()));
  await assert.rejects(host(hostDir, `ws://127.0.0.1:${unavailable.port}`), /ECONNREFUSED/);
  const running = await host(hostDir,url);
  await assert.rejects(host(hostDir,url),/EEXIST/);
  let seen: Message[] = [];
  let ui = connect(url,secret,m => seen.push(m)); await ui.ready;
  async function wait(predicate: (m: Message) => boolean) {
    for (let i=0;i<160;i++) { const m = seen.find(predicate); if (m) return m; await delay(25); }
    throw Error('Timed out waiting for relay receipt');
  }
  async function request(m: Message) { ui.send(m); return wait(r => r.type === 'receipt' && r.body.operation === m.id); }
  try {
    await wait(m => m.type === 'inventory');
    const wrong = await request(message('start','test-host',publicKey(newKey()),0)); assert.equal(wrong.body.result,'not-authority');
    const start = message('start','test-host',agent,0);
    assert.equal((await request(start)).body.result,'accepted');
    await wait(m => m.type === 'inventory' && m.body.phase === 'running');
    // Exercise the actual terminal executable twice, independently of the host.
    const identity = join(dir,'identity.json'); writePrivate(identity,{secret});
    for (let session = 0; session < 2; session++) {
      const terminal = spawn(process.execPath,['src/cli.ts','tui',identity,url],{stdio:['pipe','pipe','pipe']});
      let output = ''; let stage = 0;
      terminal.stdout.on('data', chunk => {
        output += chunk.toString();
        if (output.endsWith('beehive> ')) {
          const commands = ['hosts','select test-host','show','quit'];
          const command = commands[stage++];
          if (command) setTimeout(() => terminal.stdin.write(command+'\n'),100);
        }
      });
      const code = await new Promise(resolve => terminal.once('exit',resolve));
      assert.equal(code,0); assert.match(output,/actualRun/); assert.match(output,/running/); assert.doesNotMatch(output,/UNKNOWN/);
    }
    ui.close(); seen = [];
    ui = connect(url,secret,m => seen.push(m)); await ui.ready;
    await wait(m => m.type === 'inventory' && m.body.phase === 'running');
    assert.equal((await request(start)).revision,1);
    assert.throws(() => parseMessage({...start,id:'__proto__'}));
    const conflict = {...start,body:{unexpected:true}}; ui.send(conflict);
    await wait(m => m.type === 'receipt' && m.body.result === 'operation-id-conflict');
    assert.equal((await request(message('stop','test-host',agent,0))).body.result,'revision-conflict');
    assert.equal((await request(message('save','test-host',agent,1,{model:'fixture-model',workspace:dir,profile:'default'}))).body.result,'saved; running configuration unchanged');
    assert.equal((await request(message('stop','test-host',agent,2))).body.result,'accepted');
    await wait(m => m.type === 'inventory' && m.body.phase === 'stopped' && m.revision === 3);
    assert.ok(!readFileSync(log,'utf8').includes('fixture-model'));
    assert.ok(!readFileSync(log,'utf8').includes(agentSecret));
    await running.close();
    const restarted = await host(hostDir,url);
    try { seen = []; ui.send(message('inspect','test-host',agent)); await wait(m => m.type === 'inventory' && m.body.phase === 'stopped' && m.revision === 3); }
    finally { await restarted.close(); }
    const setupPath = join(hostDir,'setup.json');
    const setup = JSON.parse(readFileSync(setupPath,'utf8'));
    writePrivate(setupPath,{...setup,agentSecret:newKey()});
    await assert.rejects(host(hostDir,url),/Saved ownership/);
  } catch (e) { console.error(e); throw e; } finally {
    try { await running.close(); } catch {}
    ui.close(); for (const c of server.clients) c.terminate();
    await new Promise<void>(resolve => server.close(() => resolve()));
    rmSync(dir,{recursive:true,force:true});
  }
});

test('quarantined leader exit retains Stop and close teardown for owned descendants', async () => {
  for (const action of ['stop', 'close']) {
    const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-orphan-')));
    const secret = newKey(); const agentSecret = newKey(); const agent = publicKey(agentSecret);
    const hd = join(dir, 'host'); mkdirSync(hd);
    writePrivate(join(hd, 'setup.json'), { host: 'orphan-host', ownerSecret: secret, agentSecret, runner: process.execPath, args: [resolve('test/orphan-fixture.ts')], workspace: dir, mode: 'fixture' });
    const server = await relay(0, publicKey(secret), join(dir, 'relay.json'));
    const address = server.address(); assert.ok(address && typeof address !== 'string');
    const url = `ws://127.0.0.1:${address.port}`;
    const h = await host(hd, url); const seen: Message[] = [];
    const ui = connect(url, secret, m => seen.push(m)); await ui.ready;
    async function wait(predicate: (m: Message) => boolean) {
      for (let i = 0; i < 200; i++) { const m = seen.find(predicate); if (m) return m; await delay(25); }
      throw Error('Missing quarantine/teardown receipt');
    }
    try {
      ui.send(message('start', 'orphan-host', agent, 0));
      await wait(m => m.type === 'inventory' && m.body.phase === 'quarantined');
      const pid = Number(readFileSync(join(dir, 'descendant.pid'), 'utf8'));
      process.kill(pid, 0);
      const anchor = Number(readFileSync(join(dir, 'anchor.pid'), 'utf8'));
      assert.notEqual(anchor, process.pid); process.kill(anchor, 0); // Live anchor pins the group after runner exit.
      if (action === 'stop') {
        const stop = message('stop', 'orphan-host', agent, 1); ui.send(stop);
        const receipt = await wait(m => m.type === 'receipt' && m.body.operation === stop.id);
        assert.equal(receipt.body.result, 'accepted');
      } else await h.close();
      assert.throws(() => process.kill(pid, 0), (e: unknown) => (e as NodeJS.ErrnoException).code === 'ESRCH');
    } finally {
      await h.close(); ui.close(); for (const c of server.clients) c.terminate();
      await new Promise<void>(resolve => server.close(() => resolve())); rmSync(dir, { recursive: true, force: true });
    }
  }
});
