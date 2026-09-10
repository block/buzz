import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { existsSync, mkdtempSync, realpathSync, mkdirSync, readFileSync, statSync, rmSync, rmdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { newKey, publicKey, message, type Message } from '../src/protocol.ts';
import { readPrivate, writePrivate } from '../src/storage.ts';
import { bindingFingerprint, host, provision } from '../src/host.ts';
import { relay } from '../src/relay.ts';
import { connect } from '../src/client.ts';
import { installationSlots, bindingConfirmation, retireHarnessBinding, addHarnessBinding, addSlot, migrateSlots, removeSlotKey } from '../src/slots.ts';
import { createGenesis } from '../src/assignment.ts';
import { hostReady } from './host-driver.ts';
import { provisionSetup } from './provision.ts';
import { profileRevision } from '../src/profiles.ts';

async function until(check: () => boolean) {
  for (let i = 0; i < 600; i++) { if (check()) return; await delay(20); }
  throw Error('Missing named-configuration evidence');
}
type Step = { prompt: string; answer: string; gate?: () => boolean; observedRevision?: number };
async function terminal(args: string[], steps: Step[], expectedCode = 0) {
  const child = spawn(process.execPath, ['src/cli.ts', ...args], { stdio: ['pipe', 'pipe', 'pipe'] });
  let output = '', error = '', cursor = 0, index = 0, pending = false;
  child.stderr.on('data', c => { error += c.toString(); });
  let failure: unknown;
  child.stdout.on('data', c => {
    output += c.toString(); const step = steps[index];
    if (!step || pending) return;
    const position = output.indexOf(step.prompt, cursor); if (position < 0) return;
    pending = true;
    void (async () => {
      if (step.gate) await until(step.gate);
      if (step.observedRevision !== undefined) {
        // Journal commit is not proof the remote TUI has consumed inventory.
        const deadline = Date.now() + 8000;
        for (;;) {
          const from = output.length; child.stdin.write('show\n');
          await until(() => output.indexOf('beehive> ', from) >= 0);
          if (output.slice(from).includes(`\n  "revision": ${step.observedRevision},\n`)) break;
          assert.ok(Date.now() < deadline, 'TUI did not observe committed revision');
          await delay(20);
        }
      }
      await delay(80); cursor = output.length; index++; pending = false;
      child.stdin.write(`${step.answer}\n`);
    })().catch(e => { failure = e; child.kill('SIGTERM'); });
  });
  const timer = setTimeout(() => child.kill('SIGTERM'), 25000);
  try {
    const [code] = await once(child, 'exit');
    if (failure) throw new Error(`TUI step ${index} (${steps[index]?.answer}): ${String(failure)}\n${error}\n${output.slice(-24000)}`);
    assert.equal(code, expectedCode, `${error}\n${output}`); assert.equal(index, steps.length, output);
    return output;
  } finally { clearTimeout(timer); if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM'); }
}

for (const action of ['retire-binding', 'replace-binding']) for (const stale of [false, true]) test(`${action}: coherent affected sibling preview and ${stale ? 'post-preview mutation rejection' : 'unchanged confirmation'}`, { timeout: 15000 }, async () => {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), 'bh-binding-preview-')));
  try {
    const ownerSecret = newKey(), agentSecret = newKey();
    provisionSetup(join(dir, 'setup.json'), { host: 'preview', ownerSecret, agentSecret, runner: realpathSync(process.execPath), args: [resolve('test/runner.ts')], workspace: dir, mode: 'fixture' });
    migrateSlots(dir);
    const original = installationSlots(dir)[0]!;
    const originalJournal = readFileSync(original.path);
    const oldFingerprint = bindingFingerprint(original.setup);
    const siblingSecret = newKey(), sibling = publicKey(siblingSecret);
    let retained: Buffer, manifest: Buffer;
    const afterPreview = () => {
      assert.equal(existsSync(join(dir, 'host.lock')), false, 'no lock across human think time');
      if (stale) {
        const secret = newKey();
        addSlot(dir, secret, createGenesis(publicKey(ownerSecret), publicKey(secret), 'preview'));
      }
      manifest = readFileSync(join(dir, 'setup.json'));
      return true;
    };
    const output = await terminal(['local-setup', dir], [
      { prompt: 'Local action [', answer: action, gate: () => {
        addSlot(dir, siblingSecret, createGenesis(publicKey(ownerSecret), sibling, 'preview'));
        retained = readFileSync(installationSlots(dir).find(e => e.agent === sibling)!.path);
        return true;
      } },
      { prompt: 'Existing binding ID to reuse: ', answer: 'default' },
      ...(action === 'retire-binding' ? [
        { prompt: 'Retire default for new selection/execution, preserving history? [yes/no]: ', answer: 'yes', gate: afterPreview },
      ] : [
        { prompt: 'NEW immutable binding ID: ', answer: 'replacement', gate: afterPreview },
        { prompt: 'Absolute compatible executable: ', answer: realpathSync(process.execPath) },
        { prompt: 'Allowed workspace (absolute directory): ', answer: dir },
        { prompt: 'Absolute fixture TypeScript script: ', answer: resolve('test/runner.ts') },
        { prompt: 'Retire default and replace with replacement, without selecting it? [yes/no]: ', answer: 'yes' },
        { prompt: 'Save NEW binding only (no selection, key change or restart)? [yes/no]: ', answer: 'yes' },
      ]),
    ], stale ? 1 : 0);
    assert.ok(output.includes(`Agent ${sibling}: affected choices default; selected true`), 'new affected sibling must be displayed');
    const current = installationSlots(dir)[0]!;
    assert.equal(bindingFingerprint(current.bindings.default!), oldFingerprint);
    assert.deepEqual(readFileSync(original.path), originalJournal);
    assert.deepEqual(readFileSync(installationSlots(dir).find(e => e.agent === sibling)!.path), retained!);
    if (stale) {
      assert.deepEqual(readFileSync(join(dir, 'setup.json')), manifest!, 'stale confirmation must be inert');
      assert.equal(current.retiredBindings?.default, undefined);
      assert.equal(current.bindings.replacement, undefined);
    } else {
      assert.equal(current.retiredBindings?.default, oldFingerprint);
      if (action === 'replace-binding') assert.ok(current.bindings.replacement);
    }
  } finally { rmSync(dir, { recursive:true, force:true }); }
});
