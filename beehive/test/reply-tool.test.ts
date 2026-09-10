import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { mkdtempSync, realpathSync, writeFileSync, chmodSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import { ReplyTool, prepareReplyTool } from '../src/reply-tool.ts';

const scope = { executable: realpathSync(process.execPath), channel: 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee', parent: 'a'.repeat(64), recipient: 'b'.repeat(64) };
const absent = (pid: number) => { try { process.kill(pid, 0); return false; } catch (e) { return (e as NodeJS.ErrnoException).code === 'ESRCH'; } };

test('fixed reply MCP rejects caller authority and owns CLI descendants on success, cancel, disconnect and failure', async () => {
  for (const mode of ['success', 'cancel', 'disconnect', 'failure', 'startup', 'destination', 'mention', 'inactive']) {
    const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-tool-')));
    // Node executes this extensionless JS-subset TypeScript fixture as its fixed
    // "messages" program. Production passes the very same argv to real Buzz CLI.
    writeFileSync(join(dir, 'messages'), `
      const fs = require('node:fs'); const {spawn} = require('node:child_process');
      fs.writeFileSync('argv', JSON.stringify(process.argv.slice(2)));
      fs.writeFileSync('identity', process.env.BUZZ_PRIVATE_KEY);
      const child = spawn(process.execPath, ['-e', "process.on('SIGTERM',()=>{});setTimeout(()=>{},15000)"], {stdio:'ignore'});
      fs.writeFileSync('descendant', String(child.pid));
      process.stdin.resume();
      process.stdin.on('end', () => { ${mode === 'success' ? 'setTimeout(()=>process.exit(0),100)' : mode === 'failure' ? 'process.exit(2)' : "process.on('SIGTERM',()=>{});setTimeout(()=>process.exit(0),15000)"} });
    `);
    let plan = prepareReplyTool(scope);
    if (mode === 'startup') {
      const executable = join(dir, 'unavailable'); writeFileSync(executable, '', { mode: 0o700 });
      plan = prepareReplyTool({ ...scope, executable }); chmodSync(executable, 0o600);
    }
    let failed = false;
    const tool = new ReplyTool(join(dir, 'mcp'), plan, dir, { PATH: '/usr/bin:/bin', BUZZ_PRIVATE_KEY: 'fixture-only' }, () => { failed = true; void tool.stop().catch(() => {}); });
    await tool.ready;
    const shim = spawn(tool.descriptor.command, tool.descriptor.args, { detached: true, stdio: ['pipe', 'pipe', 'pipe'], env: { PATH: '/usr/bin:/bin' } });
    shim.stderr.resume();
    const responses = new Map<number, (r: any) => void>();
    createInterface({ input: shim.stdout }).on('line', line => { const m = JSON.parse(line); responses.get(m.id)?.(m.result); });
    let sequence = 0;
    const rpc = (method: string, params: any) => new Promise<any>((resolve, reject) => {
      const id = ++sequence; const timer = setTimeout(() => reject(Error('Fixture RPC timeout')), 3000);
      responses.set(id, r => { clearTimeout(timer); responses.delete(id); resolve(r); });
      shim.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
    });
    try {
      assert.equal((await rpc('initialize', {})).serverInfo.name, 'beehive-buzz-reply');
      assert.equal((await rpc('tools/list', {})).tools.length, 1);
      tool.setActive(mode !== 'inactive');
      const call = rpc('tools/call', { name: 'buzz_reply', arguments: { content: mode === 'mention' ? '@other' : 'fixture', ...(mode === 'destination' ? { channel: 'arbitrary' } : {}) } });
      void call.catch(() => {});
      if (['destination', 'mention', 'inactive', 'startup'].includes(mode)) {
        for (let i = 0; i < 100 && !failed; i++) await delay(10);
        assert.ok(failed); assert.ok(!existsSync(join(dir, 'argv')));
      } else {
        for (let i = 0; i < 100 && !existsSync(join(dir, 'descendant')); i++) await delay(10);
        assert.ok(existsSync(join(dir, 'descendant')));
        if (mode === 'success') assert.ok(!(await call).isError);
        else if (mode === 'cancel') tool.setActive(false);
        else if (mode === 'disconnect') shim.stdin.end();
        else { for (let i = 0; i < 100 && !failed; i++) await delay(10); assert.ok(failed); }
        assert.deepEqual(JSON.parse(readFileSync(join(dir, 'argv'), 'utf8')), ['send', '--channel', scope.channel, '--reply-to', scope.parent, '--mention', scope.recipient, '--content', '-']);
        assert.equal(readFileSync(join(dir, 'identity'), 'utf8'), 'fixture-only');
      }
      await tool.stop();
      assert.ok(absent(shim.pid!));
      if (existsSync(join(dir, 'descendant'))) assert.ok(absent(Number(readFileSync(join(dir, 'descendant'), 'utf8'))));
    } finally { await tool.stop(); rmSync(dir, { recursive: true, force: true }); }
  }
});
