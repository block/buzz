import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createInterface } from 'node:readline';
import { mkdtempSync, realpathSync, writeFileSync, chmodSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { setTimeout as delay } from 'node:timers/promises';
import { ReplyTool, prepareReplyTool } from '../src/reply-tool.ts';

const scope = { executable: realpathSync(process.execPath), channel: 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee', parent: 'a'.repeat(64), recipient: 'b'.repeat(64) };
const absent = (pid: number) => { try { process.kill(pid, 0); return false; } catch (e) { return (e as NodeJS.ErrnoException).code === 'ESRCH'; } };

test('fixed Buzz CLI MCP rejects caller authority and owns CLI descendants on success, cancel, disconnect and failure', async () => {
  for (const mode of ['success', 'cancel', 'disconnect', 'failure', 'startup', 'executable', 'identity', 'environment', 'relay', 'attestation', 'inactive']) {
    const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-tool-')));
    // Node executes this extensionless JS-subset TypeScript fixture as its fixed
    // "messages" program. Production passes the very same argv to real Buzz CLI.
    writeFileSync(join(dir, 'messages'), `
      const fs = require('node:fs'); const {spawn} = require('node:child_process');
      fs.writeFileSync('argv', JSON.stringify(process.argv.slice(2)));
      fs.writeFileSync('identity', process.env.BUZZ_PRIVATE_KEY);
      const child = spawn(process.execPath, ['-e', "process.on('SIGTERM',()=>{});setTimeout(()=>{},15000)"], {stdio:'ignore'});
      child.once('error', error => fs.writeFileSync('descendant-spawn-error', String(error.code)));
      child.once('spawn', () => {
        if (!Number.isSafeInteger(child.pid) || child.pid <= 0) throw Error('Invalid fixture descendant PID');
        fs.writeFileSync('descendant-spawned', JSON.stringify({ pid: child.pid, parent: process.pid }));
        fs.writeFileSync('descendant.tmp', String(child.pid));
        fs.renameSync('descendant.tmp', 'descendant');
      });
      process.stdin.resume();
      process.stdin.on('end', () => { ${mode === 'success' ? 'setTimeout(()=>process.exit(0),100)' : mode === 'failure' ? 'process.exit(2)' : "process.on('SIGTERM',()=>{});setTimeout(()=>process.exit(0),15000)"} });
    `);
    assert.throws(() => prepareReplyTool(scope), /reprovisioning/);
    let plan = prepareReplyTool({ executable: scope.executable });
    if (mode === 'startup') {
      const executable = join(dir, 'unavailable'); writeFileSync(executable, '', { mode: 0o700 });
      plan = prepareReplyTool({ executable }); chmodSync(executable, 0o600);
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
      assert.equal((await rpc('initialize', {})).serverInfo.name, 'beehive-buzz');
      assert.equal((await rpc('tools/list', {})).tools.length, 1);
      tool.setActive(mode !== 'inactive');
      const call = rpc('tools/call', { name: 'buzz', arguments: { argv: mode === 'identity' ? ['--private-key=foreign'] : mode === 'relay' ? ['--relay', 'ws://foreign.invalid'] : mode === 'attestation' ? ['--auth-tag=foreign'] : ['messages', 'send', '--channel', scope.channel, '--reply-to', scope.parent, '--mention', scope.recipient, '--content', '-'], stdin: 'fixture', ...(mode === 'executable' ? { executable: '/bin/sh' } : mode === 'environment' ? { env: { BUZZ_PRIVATE_KEY: 'foreign' } } : {}) } });
      void call.catch(() => {});
      if (['executable', 'identity', 'environment', 'relay', 'attestation', 'inactive', 'startup'].includes(mode)) {
        for (let i = 0; i < 100 && !failed; i++) await delay(10);
        assert.ok(failed); assert.ok(!existsSync(join(dir, 'argv')));
      } else {
        for (let i = 0; i < 100 && !existsSync(join(dir, 'descendant')); i++) await delay(10);
        assert.ok(existsSync(join(dir, 'descendant')), 'successful descendant spawn required');
        const descendant = Number(readFileSync(join(dir, 'descendant'), 'utf8'));
        assert.ok(Number.isSafeInteger(descendant) && descendant > 0, 'positive live-provenance fixture PID required');
        if (mode === 'success') assert.ok(!(await call).isError);
        else if (mode === 'cancel') tool.setActive(false);
        else if (mode === 'disconnect') shim.stdin.end();
        else { assert.ok((await call).isError); assert.equal(failed, false); }
        assert.deepEqual(JSON.parse(readFileSync(join(dir, 'argv'), 'utf8')), ['send', '--channel', scope.channel, '--reply-to', scope.parent, '--mention', scope.recipient, '--content', '-']);
        assert.equal(readFileSync(join(dir, 'identity'), 'utf8'), 'fixture-only');
      }
      await tool.stop();
      assert.ok(absent(shim.pid!));
      if (existsSync(join(dir, 'descendant'))) {
        const raw = readFileSync(join(dir, 'descendant'), 'utf8'), pid = Number(raw);
        // Preserve the original assertion. A recurrence must distinguish an invalid
        // fixture PID from a visible PID; numeric identity alone is not ownership.
        let errno = 'visible';
        try { process.kill(pid, 0); } catch (error) { errno = String((error as NodeJS.ErrnoException).code); }
        if (errno !== 'ESRCH') {
          const read = (name: string) => existsSync(join(dir, name)) ? readFileSync(join(dir, name), 'utf8') : null;
          const processState = Number.isSafeInteger(pid) && pid > 0
            ? spawnSync('/bin/ps', ['-o','state=,pgid=,ppid=,comm=','-p',String(pid)], { encoding:'utf8' }).stdout : null;
          console.error('Descendant exit observation', JSON.stringify({ mode, raw, pid, errno, spawned: read('descendant-spawned'), spawnError: read('descendant-spawn-error'), processState }));
        }
        assert.equal(errno, 'ESRCH', 'owned descendant must be absent after Stop (see failure capture)');
      }
    } finally { await tool.stop(); rmSync(dir, { recursive: true, force: true }); }
  }
});
