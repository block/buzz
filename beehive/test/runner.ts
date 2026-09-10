import { spawn } from 'node:child_process';
// Deterministic external fixture only: no AI, credentials, network or model claims.
if (process.env.BEEHIVE_FIXTURE !== '1') throw Error('Fixture must be explicitly enabled');
if (!process.argv.includes('--descendant')) {
  spawn(process.execPath,[process.argv[1],'--descendant'], { stdio: 'ignore' });
}
setInterval(() => {},1000);
process.on('SIGTERM',() => process.exit(0));
