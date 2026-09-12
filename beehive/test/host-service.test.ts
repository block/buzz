import { connect } from 'node:net';
import { writePrivate } from '../src/storage.ts';
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, writeFileSync, mkdirSync, existsSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { pathToFileURL } from 'node:url';
import { startService, stopService, serviceStatus, serveHost } from '../src/host-service.ts';

test('detached actual host survives client return; verified duplicate/reopen/stop; stale locks never adopted', async t => {
  const root = realpathSync(mkdtempSync('/tmp/beehive-pairing-cli-service-'));
  const directory = join(root,'host'); mkdirSync(directory,{mode:0o700});
  const entry = join(root,'service.mjs');
  writeFileSync(entry, `import {serveHost} from ${JSON.stringify(new URL('../src/host-service.ts',import.meta.url).href)};import {host} from ${JSON.stringify(new URL('../src/host.ts',import.meta.url).href)};
await serveHost(process.argv[2],process.argv[3],async()=>{const h=await host(process.argv[2],'ws://127.0.0.1',undefined,{binding:{host:'fixture-host',owner:'fixture-owner'},validate(){},connect(){return {ready:Promise.resolve(),send(){},close(){}}}});return {close:()=>h.close(),status:()=>({agents:h.agents.length,relay:'disconnected',revision:h.settingsRevision})};});`);
  assert.equal((await serviceStatus(directory)).state,'stopped');
  const first = await startService(directory,pathToFileURL(entry));
  assert.equal(first.state,'running'); assert.equal(first.agents,0);
  t.after(async () => { const current = await serviceStatus(directory); if (current.instance) await stopService(directory,current.instance); rmSync(root,{ recursive: true, force: true }); });
  assert.ok(existsSync(join(directory,'host.lock')));
  assert.equal((await startService(directory,pathToFileURL(entry))).instance,first.instance);
  // No retained client or child handle is needed after Start returns.
  assert.equal((await serviceStatus(directory)).instance,first.instance);
  await assert.rejects(stopService(directory,'f'.repeat(32)),/changed/);
  assert.equal((await serviceStatus(directory)).state,'running');
  assert.equal((await stopService(directory,first.instance!)).state,'stopped');
  assert.equal(existsSync(join(directory,'host.lock')),false);
  assert.equal((await serviceStatus(directory)).state,'stopped');
  mkdirSync(join(directory,'host.lock'));
  assert.equal((await serviceStatus(directory)).state,'unknown');
  await assert.rejects(startService(directory,pathToFileURL(entry)),/unknown/);
  assert.ok(existsSync(join(directory,'host.lock')));
});

test('startup failure does not report running or remove an unowned endpoint', async t => {
  const root = realpathSync(mkdtempSync('/tmp/beehive-pairing-cli-service-fail-')); t.after(() => rmSync(root,{recursive:true,force:true}));
  const entry = join(root,'fail.mjs');
  writeFileSync(entry,`import {serveHost} from ${JSON.stringify(new URL('../src/host-service.ts',import.meta.url).href)};try { await serveHost(process.argv[2],process.argv[3],async()=>{throw Error('fixture startup failure')}); } catch { process.exitCode=1; }`);
  await assert.rejects(startService(root,pathToFileURL(entry)),/startup failed/);
  assert.equal((await serviceStatus(root)).state,'stopped');
  writeFileSync(join(root,'service.sock'),'not our endpoint');
  assert.equal((await serviceStatus(root)).state,'unknown');
  await assert.rejects(startService(root,pathToFileURL(entry)),/unknown/);
  assert.equal(existsSync(join(root,'service.sock')),true);
});

test('failed owned teardown remains Unknown, retains reservations and never retries or accepts an unauthenticated Stop', async t => {
  const root = realpathSync(mkdtempSync('/tmp/beehive-pairing-cli-teardown-'));
  const instance = 'a'.repeat(32), token = 'b'.repeat(64);
  mkdirSync(join(root,'service.lock'),{mode:0o700});
  mkdirSync(join(root,'host.lock'),{mode:0o700});
  writePrivate(join(root,'service.json'),{version:1,instance,token,socket:join(root,'service.sock')});
  let closes = 0;
  const server = await serveHost(root,instance,async () => ({status: () => ({agents: 1, relay: 'disconnected'}), async close() { closes++; throw Error('synthetic containment failure'); }}));
  t.after(async () => { await new Promise<void>(resolve => server.close(() => resolve())); rmSync(root,{recursive:true,force:true}); });
  await new Promise<void>((resolve,reject) => {
    const socket = connect(join(root,'service.sock'));
    socket.on('error',reject); socket.on('close',() => resolve());
    socket.on('connect',() => socket.end(JSON.stringify({request:{action:'stop',instance,nonce:'c'.repeat(32)},mac:'0'.repeat(64)})+'\n'));
  });
  assert.equal(closes,0);
  assert.equal((await serviceStatus(root)).state,'running');
  const results = await Promise.all([stopService(root,instance),stopService(root,instance)]);
  assert.ok(results.every(r => r.state === 'unknown'));
  assert.equal(closes,1,'one retained teardown promise, not repeated process control');
  assert.equal((await serviceStatus(root)).state,'unknown');
  assert.equal((await stopService(root,instance)).state,'unknown');
  assert.equal(closes,1);
  await assert.rejects(startService(root),/unknown/);
  for (const path of ['service.json','service.lock','host.lock','service.sock']) assert.ok(existsSync(join(root,path)),path);
});
