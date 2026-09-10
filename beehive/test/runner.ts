import { appendFileSync } from 'node:fs';
import { spawn } from 'node:child_process';
// Deterministic external fixture only: no AI, credentials, network or model claims.
if (process.env.BEEHIVE_FIXTURE !== '1') throw Error('Fixture must be explicitly enabled');
if (!process.argv.includes('--descendant')) {
  appendFileSync('received-instructions.jsonl', JSON.stringify({ pid: process.pid, instructions: process.env.BUZZ_AGENT_SYSTEM_PROMPT ?? null }) + '\n');
  spawn(process.execPath,[process.argv[1],'--descendant'], { stdio: 'ignore' });
}
setInterval(() => {},1000);
process.on('SIGTERM',() => process.exit(0));
