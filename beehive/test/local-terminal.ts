import { spawn } from 'node:child_process';
import { once } from 'node:events';
import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';
async function until(check: () => boolean) {
  for (let i = 0; i < 600; i++) { if (check()) return; await delay(20); }
  throw Error('Missing named-configuration evidence');
}
type Step = { prompt: string; answer: string; gate?: () => boolean; observedRevision?: number };
export async function terminal(args: string[], steps: Step[], pty = false) {
  const child = spawn(pty ? '/bin/sh' : process.execPath, pty ? ['-c', 'cat | /usr/bin/script -q /dev/null "$@"', 'beehive-pty', process.execPath, 'src/cli.ts', ...args] : ['src/cli.ts', ...args], { stdio: ['pipe', 'pipe', 'pipe'] });
  let output = '', error = '', cursor = 0, index = 0, pending = false;
  child.stderr.on('data', c => { error += c.toString(); });
  let failure: unknown;
  child.stdout.on('data', c => {
    output += c.toString(); if (pty && output.includes('Local setup saved.')) child.stdin.end(); const step = steps[index];
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
    assert.equal(code, 0, `${error}\n${output}`); assert.equal(index, steps.length, output);
    return output;
  } finally { clearTimeout(timer); if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM'); }
}

