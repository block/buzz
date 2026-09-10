import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { hostReady } from './host-driver.ts';

test('host driver retains bounded stderr and actual early exit instead of readiness timeout', async () => {
  const child = spawn(process.execPath, ['-e', 'process.stderr.write("x".repeat(20000) + "startup sentinel"); process.exitCode = 7'], { stdio: ['ignore', 'pipe', 'pipe'] });
  await assert.rejects(hostReady(child, 'failed fixture'), error => {
    assert.ok(error instanceof Error);
    assert.match(error.message, /exit=7/);
    assert.match(error.message, /startup sentinel/);
    assert.ok(error.message.length < 9000);
    return true;
  });
});

test('host driver reports spawn errors without waiting for readiness', async () => {
  const child = spawn('/nonexistent-beehive-test-executable', [], { stdio: ['ignore', 'pipe', 'pipe'] });
  await assert.rejects(hostReady(child, 'spawn fixture'), /ENOENT/);
});
