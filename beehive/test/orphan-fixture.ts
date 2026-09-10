/** Non-escaping child survives its parent, to exercise owned quarantine teardown. */
import { spawn } from 'node:child_process';
import { appendFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
if (process.argv[2] === 'child') {
  // Fresh fixture only: capped lifecycle records distinguish handler installation,
  // callback delivery/write failure and self-expiry. Never used as kill authority.
  let records = 0;
  const record = (event: string, code?: string) => {
    if (process.argv[3] === 'resist' && records++ < 8) appendFileSync(join(process.cwd(), 'resistant.trace'), `${JSON.stringify({ event, pid: process.pid, at: Date.now(), cwd: process.cwd(), code })}\n`);
  };
  if (process.argv[3] === 'resist') {
    process.on('SIGTERM', () => {
      record('term-handler-entered');
      try { writeFileSync(join(process.cwd(), 'term-seen'), '1'); }
      catch (error) { record('term-write-error', (error as NodeJS.ErrnoException).code); throw error; }
      record('term-written');
    });
    record('handler-installed');
    writeFileSync(join(process.cwd(), 'resistant.ready'), '1');
  }
  setInterval(() => {}, 1000);
  setTimeout(() => { record('self-expiry'); process.exit(0); }, 15_000); // Bound failed regression runs without numeric cleanup kills.
}
else {
  const child = spawn(process.execPath, [import.meta.filename, 'child', process.argv[2] ?? 'cooperate'], { stdio: 'ignore' });
  writeFileSync(join(process.cwd(), 'descendant.pid'), String(child.pid));
  writeFileSync(join(process.cwd(), 'anchor.pid'), String(process.ppid));
  child.unref();
  setTimeout(() => process.exit(0), 500);
}
