import { nativeCredentials } from './native-credentials.ts';
import type { CredentialReference } from './credential-store.ts';

// Dedicated one-shot private pipe protocol. No errors or secrets to stderr/files.
let input = '';
process.stdin.on('data', (chunk: Buffer) => {
  if (Buffer.byteLength(input) + chunk.length > 512) process.exit(1);
  input += chunk.toString('utf8');
});
process.stdin.on('end', () => {
  try {
    const reference = JSON.parse(input) as CredentialReference; input = '';
    const secret = nativeCredentials().read(reference);
    process.stdout.end(JSON.stringify(secret === null ? { status: 'missing' } : { status: 'present', secret }));
  } catch { process.stdout.end(JSON.stringify({ status: 'failed' })); }
});
