import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, rmSync, existsSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { spawnOwned } from '../src/owned.ts';

test('parent IPC loss retains anchor until resistant descendants are forcibly stopped', async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'beehive-disconnect-')));
  const owned = spawnOwned(process.execPath, [resolve('test/orphan-fixture.ts'), 'resist'], dir, { PATH: '/usr/bin:/bin' });
  owned.child.stdout.resume(); owned.child.stderr.resume();
  try {
    await owned.ready;
    for (let i = 0; i < 100 && !existsSync(join(dir, 'resistant.ready')); i++) await delay(10);
    assert.ok(existsSync(join(dir, 'resistant.ready')));
    const descendant = Number(readFileSync(join(dir, 'descendant.pid'), 'utf8'));
    owned.child.disconnect(); // Simulates host loss, not a recovered-PID cleanup operation.
    let absent = false;
    for (let i = 0; i < 150; i++) {
      try { process.kill(-owned.child.pid!, 0); }
      catch (error) { if ((error as NodeJS.ErrnoException).code === 'ESRCH') { absent = true; break; } }
      await delay(20);
    }
    assert.ok(absent, 'IPC loss must not destroy the anchor before resistant group teardown');
    assert.equal(readFileSync(join(dir, 'term-seen'), 'utf8'), '1');
    assert.throws(() => process.kill(descendant, 0), (e: unknown) => (e as NodeJS.ErrnoException).code === 'ESRCH');
  } finally {
    if (owned.child.connected) await owned.stop();
    rmSync(dir, { recursive: true, force: true });
  }
});
