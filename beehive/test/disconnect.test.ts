import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, rmSync, existsSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { spawnOwned } from '../src/owned.ts';

test('parent IPC loss retains anchor until resistant descendants are forcibly stopped', async t => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-disconnect-')));
  const owned = spawnOwned(process.execPath, [resolve('test/orphan-fixture.ts'), 'resist'], dir, { PATH: '/usr/bin:/bin' });
  owned.child.stdout.resume(); owned.child.stderr.resume();
  let disconnectedAt: number | undefined;
  let descendant: number | undefined;
  try {
    await owned.ready;
    for (let i = 0; i < 100 && !existsSync(join(dir, 'resistant.ready')); i++) await delay(10);
    assert.ok(existsSync(join(dir, 'resistant.ready')));
    descendant = Number(readFileSync(join(dir, 'descendant.pid'), 'utf8'));
    assert.ok(Number.isSafeInteger(descendant) && descendant > 0, 'fixture must publish a positive descendant PID');
    disconnectedAt = Date.now();
    owned.child.disconnect(); // Simulates host loss, not a recovered-PID cleanup operation.
    let absent = false;
    for (let i = 0; i < 150; i++) {
      try { process.kill(-owned.child.pid!, 0); }
      catch (error) { if ((error as NodeJS.ErrnoException).code === 'ESRCH') { absent = true; break; } }
      await delay(20);
    }
    assert.ok(absent, 'IPC loss must not destroy the anchor before resistant group teardown');
    assert.equal(readFileSync(join(dir, 'term-seen'), 'utf8'), '1');
    assert.throws(() => process.kill(descendant!, 0), (e: unknown) => (e as NodeJS.ErrnoException).code === 'ESRCH');
    const trace = readFileSync(join(dir, 'resistant.trace'), 'utf8').trim().split('\n').map(line => JSON.parse(line));
    assert.deepEqual(trace.map(e => e.event), ['handler-installed', 'term-handler-entered', 'term-written']);
    assert.ok(trace.every(e => e.pid === descendant && e.cwd === dir));
    t.diagnostic(JSON.stringify({ disconnectedAt, observedAbsentAt: Date.now(), trace }));
  } catch (error) {
    // Capture before finally removes the fresh fixture. A pass cannot classify a
    // prior failure; absence of a callback does not mean its handler was absent.
    const tracePath = join(dir, 'resistant.trace');
    throw Error(`Disconnect fixture evidence ${JSON.stringify({ directory: dir, anchor: owned.child.pid, descendant, disconnectedAt, observedAt: Date.now(), exitCode: owned.child.exitCode, signalCode: owned.child.signalCode, ready: existsSync(join(dir, 'resistant.ready')), term: existsSync(join(dir, 'term-seen')), trace: existsSync(tracePath) ? readFileSync(tracePath, 'utf8').slice(-8192) : null })}`, { cause: error });
  } finally {
    if (owned.child.connected) await owned.stop();
    rmSync(dir, { recursive: true, force: true });
  }
});
