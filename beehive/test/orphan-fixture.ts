/** Non-escaping child survives its parent, to exercise owned quarantine teardown. */
import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
if (process.argv[2] === 'child') setInterval(() => {}, 1000);
else {
  const child = spawn(process.execPath, [import.meta.filename, 'child'], { stdio: 'ignore' });
  writeFileSync(join(process.cwd(), 'descendant.pid'), String(child.pid));
  writeFileSync(join(process.cwd(), 'anchor.pid'), String(process.ppid));
  child.unref();
  setTimeout(() => process.exit(0), 500);
}
