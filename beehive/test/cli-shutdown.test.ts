import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { WebSocket } from 'ws';
import { newKey, publicKey, seal, message } from '../src/protocol.ts';

for (const fault of [false, true]) test(`actual relay CLI SIGINT surfaces uncertainty=${fault}`, { timeout: 10000 }, async () => {
  const dir = mkdtempSync(join(tmpdir(), 'beehive-cli-close-'));
  const secret = newKey(), path = join(dir, 'history.json');
  const child = spawn(process.execPath, [...(fault ? ['--import', './test/relay-cli-fault.ts'] : []), 'src/cli.ts', 'relay', '0', publicKey(secret), path], { stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '', errors = ''; child.stdout.on('data', b => output = (output + b).slice(-8192)); child.stderr.on('data', b => errors = (errors + b).slice(-8192));
  const exited = once(child, 'exit');
  let ws: WebSocket | undefined;
  try {
    while (!output.includes('\n')) await once(child.stdout, 'data');
    const address = JSON.parse(output.trim().replace('Development relay listening ', ''));
    ws = new WebSocket(`ws://127.0.0.1:${address.port}`); await once(ws, 'open');
    const observed = once(ws, fault ? 'close' : 'message');
    const envelope = seal(message('inspect', 'h', 'a'), secret); ws.send(JSON.stringify(envelope));
    const result = await observed;
    if (fault) assert.equal(result[0], 1011);
    assert.deepEqual(JSON.parse(readFileSync(path, 'utf8')), [envelope], 'replacement is possible, not a durable rejection');
    child.kill('SIGINT');
    assert.deepEqual(await exited, [fault ? 1 : 0, null]);
    if (fault) assert.match(errors, /storage durability may be uncertain/); else assert.equal(errors, '');
  } finally {
    ws?.terminate();
    if (child.exitCode === null && child.signalCode === null) { child.kill('SIGTERM'); await exited; }
    rmSync(dir, { recursive: true, force: true });
  }
});
