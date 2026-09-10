import { createInterface } from 'node:readline/promises';
import { Writable } from 'node:stream';
import { stdin, stdout } from 'node:process';
import { publicKey } from './protocol.ts';

/** Local secret entry only: disable terminal echo and never route key bytes to output.
 * Piped input is supported for protected automation; no secret argv or environment. */
export async function readAgentSecret(role: 'Agent' | 'Owner' = 'Agent'): Promise<string> {
  const muted = new Writable({ write(_chunk, _encoding, done) { done(); } });
  const ui = createInterface({ input: stdin, output: muted, terminal: Boolean(stdin.isTTY) });
  try {
    stdout.write(`${role} private key (64 hex; hidden, never passed as an argument): `);
    const value = await ui.question('');
    if (!/^[0-9a-fA-F]{64}$/.test(value)) throw Error('Invalid private key; expected 64 hex characters');
    const secret = value.toLowerCase();
    try { publicKey(secret); } catch { throw Error('Invalid private key'); }
    return secret;
  } finally { ui.close(); muted.end(); stdout.write('\n'); }
}
