import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, realpathSync, writeFileSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { ConversationSession } from '../src/broker.ts';
import { newKey, publicKey } from '../src/protocol.ts';

test('broker enforces model, completion health, cancellation and owned teardown across escaped shims', async () => {
  for (const mode of ['ok', 'optional-ok', 'optional-reject', 'optional-reack-fails', 'model-config-reject', 'reject', 'wrong-model', 'conflict', 'bad-tail', 'delayed', 'cancel', 'foreign-workspace', 'injected-mcp', 'tool-scope-mismatch']) {
    const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-broker-test-')));
    const owner = publicKey(newKey());
    const executable = realpathSync(process.execPath); const runtime = join(dir, 'runtime');
    writeFileSync(runtime, `#!/bin/sh\nexec '${executable}' '${resolve('test/conversation-runtime-fixture.ts')}'\n`, { mode: 0o700 });
    writeFileSync(join(dir, 'mode'), mode);
    const session = new ConversationSession({ executable: runtime, relay: 'ws://127.0.0.1:1', ...(['injected-mcp', 'tool-scope-mismatch'].includes(mode) ? { replyTool: { executable, channel: 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee', parent: 'a'.repeat(64), recipient: owner } } : {}) }, {
      executable, args: [resolve('test/conversation-harness-fixture.ts')], workspace: dir, home: dir, configDirectory: dir,
      databricksHost: 'https://fixture.invalid', model: 'databricks-claude-haiku-4-5',
    }, newKey(), owner, 2000);
    const result = session.verify().then(value => ({ value }), error => ({ error }));
    try {
      if (mode === 'delayed' || mode === 'cancel') {
        for (let i = 0; i < 80 && !existsSync(join(dir, mode === 'cancel' ? 'cancel-observed' : 'prompt-started')); i++) await delay(20);
        assert.ok(existsSync(join(dir, mode === 'cancel' ? 'cancel-observed' : 'prompt-started')));
        session.cancel();
      }
      const r = await result;
      if (['ok', 'optional-ok', 'optional-reject'].includes(mode)) {
        if (mode === 'optional-reject') assert.equal(readFileSync(join(dir, 'config-error-forwarded'), 'utf8'), 'yes');
        assert.ok('value' in r); assert.equal(r.value.session, 'conversation-session');
        for (let i = 0; i < 40 && !existsSync(join(dir, 'reverse-rpc')); i++) await delay(20);
        assert.equal(readFileSync(join(dir, 'reverse-rpc'), 'utf8'), 'ok');
      } else assert.ok('error' in r, `${mode} must not commit evidence`);
    } finally {
      await session.stop();
      for (const file of ['runtime-pid', 'shim-pid', 'harness-pid', 'descendant-pid']) {
        if (existsSync(join(dir, file))) assert.throws(() => process.kill(Number(readFileSync(join(dir, file), 'utf8')), 0), { code: 'ESRCH' }, `${mode}: ${file}`);
      }
      rmSync(dir, { recursive: true, force: true });
    }
  }
});
