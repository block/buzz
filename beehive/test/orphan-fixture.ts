/** Non-escaping child survives its parent, to exercise owned quarantine teardown. */
import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
if (process.argv[2] === 'child') {
  if (process.argv[3] === 'resist') {
    process.on('SIGTERM', () => writeFileSync(join(process.cwd(), 'term-seen'), '1'));
    writeFileSync(join(process.cwd(), 'resistant.ready'), '1');
  }
  setInterval(() => {}, 1000);
  setTimeout(() => process.exit(0), 15_000); // Bound failed regression runs without numeric cleanup kills.
}
else {
  const child = spawn(process.execPath, [import.meta.filename, 'child', process.argv[2] ?? 'cooperate'], { stdio: 'ignore' });
  writeFileSync(join(process.cwd(), 'descendant.pid'), String(child.pid));
  writeFileSync(join(process.cwd(), 'anchor.pid'), String(process.ppid));
  child.unref();
  setTimeout(() => process.exit(0), 500);
}
