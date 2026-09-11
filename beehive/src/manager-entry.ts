import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { homedir } from 'node:os';
import { ManagerController } from './manager-controller.ts';
import type { Readable, Writable } from 'node:stream';

if (process.versions.node !== '24.15.0') throw Error('Beehive requires Node 24.15.0. Use the packaged Beehive launcher.');
const bun = process.env.BEEHIVE_BUN ?? fileURLToPath(new URL('../runtime/bun', import.meta.url));
const child = spawn(bun, [fileURLToPath(new URL('./manager-view.ts', import.meta.url))], {
  stdio: ['inherit', 'inherit', 'inherit', 'pipe', 'pipe'],
  env: { PATH: '/usr/bin:/bin', TERM: process.env.TERM ?? 'xterm-256color', COLORTERM: process.env.COLORTERM ?? '', ...(process.env.HOME ? { HOME: process.env.HOME } : {}) },
});
const output = child.stdio[3] as Writable;
const input = createInterface({ input: child.stdio[4] as Readable });
let closed = false;
const send = (value: object) => { if (!closed && !output.destroyed) output.write(JSON.stringify(value) + '\n'); };
const controller = new ManagerController(homedir(), snapshot => send({ snapshot }));
output.on('error', () => controller.close());
input.on('line', line => {
  if (line.length > 131072) { controller.cancel(); return; }
  try {
    const value = JSON.parse(line);
    if (value.quit) controller.close();
    else if (value.cancel) controller.cancel();
    else void controller.request(value).then(() => send({ complete: value.id }));
  } catch { controller.cancel(); }
});
const refresh = setInterval(() => controller.refresh(), 2000);
child.once('spawn', () => controller.refresh());
child.once('error', () => { console.error('Beehive could not start its terminal interface. The packaged Bun runtime is unavailable.'); process.exitCode = 1; });
child.once('close', code => { closed = true; clearInterval(refresh); controller.close(); input.close(); process.exitCode = code ?? 1; });
for (const signal of ['SIGINT','SIGTERM'] as const) process.on(signal, () => { controller.close(); child.kill(signal); });
