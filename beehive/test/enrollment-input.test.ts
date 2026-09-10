import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { newKey, publicKey } from '../src/protocol.ts';
import { readHostIdentity, requireHostEnrollment } from '../src/host-identity.ts';
import { pairingFingerprint } from '../src/host-registration.ts';

async function wizard(directory: string, steps: [string, string][]): Promise<string> {
  const env = { ...process.env };
  delete env.BUZZ_PRIVATE_KEY; delete env.BUZZ_AUTH_TAG; delete env.BUZZ_RELAY_URL;
  const child = spawn(process.execPath, [fileURLToPath(new URL('../src/cli.ts', import.meta.url)), 'setup', directory], { env, stdio: ['pipe', 'pipe', 'pipe'] });
  let output = '', cursor = 0, next = 0;
  const timer = setTimeout(() => child.kill(), 10000);
  child.stdout.on('data', data => {
    output += data.toString();
    if (next < steps.length) {
      const [prompt, answer] = steps[next]!;
      const index = output.indexOf(prompt, cursor);
      if (index !== -1) { cursor = index + prompt.length; next++; child.stdin.write(`${answer}\n`); }
    }
  });
  child.stderr.on('data', data => { output += data.toString(); });
  try {
    const code = await new Promise<number | null>((resolve, reject) => { child.on('error', reject); child.on('exit', resolve); });
    assert.equal(code, 0, output); assert.equal(next, steps.length, output);
    return output;
  } finally { clearTimeout(timer); child.stdin.end(); }
}

test('actual standalone CLI private pairing, existing-owner approval and bootstrap return stay offline', async () => {
  const root = mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-'));
  const host = join(root, 'host'), ownerContext = join(root, 'owner-context');
  const request = join(root, 'request.json'), approval = join(root, 'approval.json');
  const ownerSecret = newKey();
  try {
    const created = await wizard(host, [
      ['Enrollment [', 'create'], ['Host label: ', 'desktop'], ['Owner PUBLIC key (hex): ', publicKey(ownerSecret)],
      ['Relay URL: ', 'wss://example.invalid'], ['NEW private pairing file: ', request],
    ]);
    const original = readHostIdentity(host);
    assert.ok(created.includes(pairingFingerprint(original.pairing)));
    assert.equal(original.registration, null);
    const signed = await wizard(ownerContext, [
      ['Enrollment [', 'approve'], ['Private pairing file: ', request], ['[yes/no]: ', 'yes'],
      ['Registration expiration (Unix seconds): ', String(Math.floor(Date.now() / 1000) + 300)],
      ['NEW private approval file: ', approval], ['Owner private key (64 hex; hidden, never passed as an argument): ', ownerSecret],
    ]);
    assert.ok(signed.includes(pairingFingerprint(original.pairing)));
    assert.ok(!signed.includes(ownerSecret));
    const imported = await wizard(host, [['Enrollment [', 'import'], ['Private approval file: ', approval]]);
    assert.ok(imported.includes('Pending/no network'));
    const enrolled = readHostIdentity(host);
    assert.equal(enrolled.secret, original.secret);
    assert.deepEqual(enrolled.pairing, original.pairing);
    assert.throws(() => requireHostEnrollment(enrolled), /relay admission pending/);
    assert.deepEqual(readdirSync(host), ['host-identity.json']);
    for (const path of [request, approval, join(host, 'host-identity.json')]) assert.ok(!readFileSync(path, 'utf8').includes(ownerSecret));
  } finally { rmSync(root, { recursive: true, force: true }); }
});
