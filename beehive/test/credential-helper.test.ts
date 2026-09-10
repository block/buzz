import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { credentialHelperReader } from '../src/credential-helper.ts';
import { credentialReference, systemCredentials } from '../src/credential-store.ts';
import { provisionCredentialSlot } from '../src/credential-slots.ts';
import { host, validateSetup } from '../src/host.ts';
import { createGenesis } from '../src/assignment.ts';
import { newKey, publicKey, message, type Message } from '../src/protocol.ts';
import { memoryCredentials } from './credential-fixture.ts';

async function until(predicate: () => boolean) {
  for (let i = 0; i < 200; i++) { if (predicate()) return; await delay(10); }
  throw Error('Fixture evidence missing');
}
function blocker(directory: string) {
  const pidFile = join(directory, 'helper.pid'), script = join(directory, 'helper.mjs');
  // PID-only fixture evidence, never credential material. Blocking JS cannot run
  // an abort callback, approximating a native call's event-loop unresponsiveness.
  writeFileSync(script, `import {writeFileSync,renameSync} from 'node:fs';\nwriteFileSync(${JSON.stringify(pidFile + '.pending')},String(process.pid));\nrenameSync(${JSON.stringify(pidFile + '.pending')},${JSON.stringify(pidFile)});\nAtomics.wait(new Int32Array(new SharedArrayBuffer(4)),0,0);\n`);
  return { pidFile, helper: pathToFileURL(script) };
}
function gone(pidFile: string) {
  const pid = Number(readFileSync(pidFile, 'utf8'));
  assert.ok(Number.isSafeInteger(pid) && pid > 1, 'Atomic fixture readiness must contain the owned PID, not an empty file/process group');
  assert.throws(() => process.kill(pid, 0), (e: NodeJS.ErrnoException) => e.code === 'ESRCH');
}

test('one-shot blocked helper abort/deadline waits for actual exit, bounded concurrency, no OS calls', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'beehive-helper-'));
  try {
    const fixture = blocker(directory), ref = credentialReference('agent', publicKey(newKey()));
    const read = credentialHelperReader({ operatorApproved: true, helper: fixture.helper, timeoutMs: 1000 });
    const controller = new AbortController();
    const pending = read(ref, controller.signal);
    const rejected = assert.rejects(pending, /cancelled/);
    await until(() => existsSync(fixture.pidFile));
    await assert.rejects(read(ref), /busy/);
    controller.abort(); await rejected; gone(fixture.pidFile);
    await assert.rejects(read(ref), /timed out.*possibly permission-prompt blocked/); gone(fixture.pidFile);
    await assert.rejects(systemCredentials.readAsync!(ref), /owner-present/);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

test('v3 host Stop and close cancel a pending credential child before any spawn; late fixture secrets are fenced', async () => {
  for (const action of ['stop', 'close', 'late'] as const) {
    const directory = mkdtempSync(join(tmpdir(), 'beehive-helper-host-'));
    const backend = memoryCredentials(), owner = newKey(), secret = newKey(), agent = publicKey(secret);
    const setup = validateSetup({ host: 'helper-host', ownerPublic: publicKey(owner), mode: 'fixture', runner: process.execPath, args: [resolve('test/runner.ts')], workspace: directory });
    provisionCredentialSlot(directory, setup, secret, createGenesis(publicKey(owner), agent, setup.host), backend);
    const fixture = blocker(directory), read = credentialHelperReader({ operatorApproved: true, helper: fixture.helper });
    let hydration = true, pending = false, late: (() => void) | undefined;
    const seen: Message[] = []; let receive!: (m: Message) => void;
    const h = await host(directory, 'ws://fixture.invalid', undefined, {
      binding: { host: setup.host, owner: publicKey(owner) }, validate() {},
      connect(_url, _secret, callback) { receive = callback; return { ready: Promise.resolve(), send(m) { seen.push(m); }, close() {} }; },
    }, { ...backend, async readAsync(ref, signal) {
      if (hydration) return backend.read(ref);
      pending = true;
      if (action === 'late') { await new Promise<void>(r => { late = r; }); return secret; }
      return read(ref, signal);
    } });
    hydration = false;
    try {
      const start = message('start', setup.host, agent, 0); receive(start);
      await until(() => pending && (action === 'late' || existsSync(fixture.pidFile)));
      if (action === 'close') await h.close();
      else {
        const stop = message('stop', setup.host, agent, 0); receive(stop); late?.();
        await until(() => seen.some(m => m.type === 'receipt' && m.body.operation === stop.id));
        assert.equal(seen.find(m => m.body.operation === stop.id)?.body.result, 'accepted');
      }
      if (action !== 'late') gone(fixture.pidFile);
      const state = JSON.parse(readFileSync(join(directory, 'agents', agent, 'journal.json'), 'utf8'));
      assert.equal(state.phase, 'stopped'); assert.equal(state.actual, null);
      assert.notEqual(state.operations[start.id].reply.body.result, 'accepted');
      assert.equal(Object.keys(state.runs ?? {}).length, 0);
      assert.ok(!seen.some(m => m.type === 'inventory' && m.body.phase === 'running'));
    } finally { late?.(); await h.close(); rmSync(directory, { recursive: true, force: true }); }
  }
});
