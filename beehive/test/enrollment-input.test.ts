import { isolatedFileCredentials } from './isolated-file-credentials.ts';
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, readdirSync, rmSync, copyFileSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { newKey, publicKey } from '../src/protocol.ts';
import { readHostIdentity, requireHostEnrollment } from '../src/host-identity.ts';
import { setupPath } from '../src/setup-path.ts';
import { homedir } from 'node:os';
import { createGenesis } from '../src/assignment.ts';
import { writePrivate } from '../src/storage.ts';
import { pairingFingerprint, registerHost } from '../src/host-registration.ts';

async function wizard(directory: string, steps: [string, string][], credentialFile: string, expectedCode = 0, home?: string, command = 'legacy-enrollment', extra: string[] = []): Promise<string> {
  const env: NodeJS.ProcessEnv = { ...process.env, ...(home ? { HOME: home } : {}), BEEHIVE_TEST_CREDENTIAL_FILE: credentialFile };
  delete env.BUZZ_PRIVATE_KEY; delete env.BUZZ_AUTH_TAG; delete env.BUZZ_RELAY_URL;
  const child = spawn(process.execPath, ['--import', fileURLToPath(new URL('./isolated-credentials-loader.ts', import.meta.url)), fileURLToPath(new URL('../src/cli.ts', import.meta.url)), command, directory, ...extra], { env, stdio: ['pipe', 'pipe', 'pipe'] });
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
    assert.equal(code, expectedCode, output); assert.equal(next, steps.length, output);
    return output;
  } finally { clearTimeout(timer); child.stdin.end(); }
}

test('actual standalone CLI private pairing, existing-owner approval and bootstrap return stay offline', async () => {
  const root = mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-'));
  const host = join(root, 'host'), ownerContext = join(root, 'owner-context');
  let request: string, approval: string;
  const ownerSecret = newKey();
  const credentialFile = join(root, 'isolated-fixture-secrets.json');
  const credentials = isolatedFileCredentials(credentialFile);
  try {
    const created = await wizard(host, [
      ['Enrollment [', 'create'], ['Computer name [', ''], ['Owner PUBLIC key (hex,', publicKey(ownerSecret)],
      ['Management relay URL (', 'wss://example.invalid'],
    ], credentialFile);
    request = join(host, 'exchange', readdirSync(join(host, 'exchange'))[0]!);
    assert.match(created, /Request saved:/);
    assert.ok(!created.includes('output path (~ supported):'));
    const original = readHostIdentity(host, credentials);
    const identityBytes = readFileSync(join(host, 'host-identity.json'));
    // Re-export reads only retained public state, not even the isolated credential store.
    const poison = join(root, 'must-not-read.json'); writeFileSync(poison, 'invalid json');
    const resumed = await wizard(host, [['Enrollment [', '']], poison);
    assert.match(resumed, /Existing computer identity retained/);
    assert.equal(readdirSync(join(host, 'exchange')).length, 2);
    assert.deepEqual(readFileSync(join(host, 'host-identity.json')), identityBytes);
    assert.deepEqual(JSON.parse(readFileSync(request, 'utf8')), original.pairing);
    const explicit = await wizard(host, [['Enrollment [', 'export-to'], ['output path (~ supported): ', '~/manual-request.json']], poison, 0, root);
    assert.match(explicit, /manual-request.json/);
    const manualBytes = readFileSync(join(root, 'manual-request.json'));
    const refused = await wizard(host, [['Enrollment [', 'export-to'], ['output path (~ supported): ', '~/manual-request.json']], poison, 1, root);
    assert.match(refused, /EEXIST/);
    assert.deepEqual(readFileSync(join(root, 'manual-request.json')), manualBytes);
    const missing = await wizard(host, [['Enrollment [', 'import'], ['Private approval file (', '']], poison, 1);
    assert.match(missing, /Transfer the owner-signed approval/);
    assert.ok(created.includes(pairingFingerprint(original.pairing)));
    assert.equal(original.registration, null);
    const signed = await wizard(ownerContext, [
      ['Enrollment [', 'approve'], ['Private pairing file (', '~/manual-request.json'], ['[yes/no]: ', 'yes'],
      ['Approval validity (', '7d'], ['Owner private key (64 hex; hidden, never passed as an argument): ', ownerSecret],
    ], credentialFile, 0, root);
    approval = join(ownerContext, 'exchange', readdirSync(join(ownerContext, 'exchange'))[0]!);
    assert.ok(signed.includes(pairingFingerprint(original.pairing)));
    assert.ok(!signed.includes(ownerSecret));
    const catalogOutput = await wizard(approval, [], credentialFile, 0, undefined, 'catalog');
    assert.match(catalogOutput, /Next on owner computer: beehive tui/);
    const defaultApproval = join(host, 'exchange', `approval-${pairingFingerprint(original.pairing)}.json`);
    // A matching filename does not authorize anything.
    writePrivate(defaultApproval, registerHost({ ...original.pairing, nonce: newKey() }, ownerSecret, Math.floor(Date.now() / 1000) + 300), true);
    await wizard(host, [['Enrollment [', 'import'], ['Private approval file (', '']], credentialFile, 1);
    assert.deepEqual(readFileSync(join(host, 'host-identity.json')), identityBytes);
    copyFileSync(approval, defaultApproval);
    const imported = await wizard('~/host', [['Enrollment [', 'import'], ['Private approval file (', '']], credentialFile, 0, root);
    assert.ok(imported.includes('Pending/no network'));
    const enrolled = readHostIdentity(host, credentials);
    assert.equal(enrolled.secret, original.secret);
    assert.ok(!readFileSync(join(host, 'host-identity.json'), 'utf8').includes(original.secret));
    assert.deepEqual(enrolled.pairing, original.pairing);
    assert.throws(() => requireHostEnrollment(enrolled), /relay admission pending/);
    assert.deepEqual(readdirSync(host), ['exchange', 'host-identity.json']);
    assert.equal(statSync(join(host, 'exchange')).mode & 0o777, 0o700);
    for (const file of [request, approval, defaultApproval]) assert.equal(statSync(file).mode & 0o777, 0o600);
    const ownerOnHost = await wizard(host, [['Enrollment [', 'approve']], poison, 1);
    assert.match(ownerOnHost, /Approve on the trusted owner computer/);
    assert.ok(!ownerOnHost.includes('Owner private key'));
    const agentSecret = newKey();
    writePrivate(join(root, 'binding.json'), { mode: 'fixture', runner: process.execPath, args: [fileURLToPath(new URL('./runner.ts', import.meta.url))], workspace: root }, true);
    writePrivate(join(root, 'genesis.json'), createGenesis(publicKey(ownerSecret), publicKey(agentSecret), original.pairing.host), true);
    const provisioned = await wizard('~/host', [['Agent private key (64 hex; hidden, never passed as an argument): ', agentSecret]], credentialFile, 0, root, 'provision-agent', ['~/binding.json', '~/genesis.json']);
    assert.match(provisioned, /Provisioned STOPPED/);
    assert.ok(!provisioned.includes(agentSecret));
    assert.ok(!readFileSync(join(host, 'setup.json'), 'utf8').includes(agentSecret));
    for (const path of [request, approval, join(host, 'host-identity.json')]) assert.ok(!readFileSync(path, 'utf8').includes(ownerSecret));
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('setup paths expand only conventional home spelling', () => {
  assert.equal(setupPath('~'), homedir());
  assert.equal(setupPath('~/a b'), join(homedir(), 'a b'));
  assert.throws(() => setupPath('~someone/file'), /~user/);
});
