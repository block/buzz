import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawn } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
import { createGenesis } from '../src/assignment.ts';
import { provisionCredentialSlot, addCredentialSlot, orphanCredentialSlots } from '../src/credential-slots.ts';
import { installationSlots } from '../src/slots.ts';
import { isolatedFileCredentials } from './isolated-file-credentials.ts';
import { newKey, publicKey } from '../src/protocol.ts';
import { readPrivate, writePrivate } from '../src/storage.ts';
import { validateSetup } from '../src/host.ts';

/** Actual standalone v3 CLI journeys for interrupted added-slot recovery, using the
 * existing explicit isolated-file credential loader. Synthetic fixtures only; no OS
 * credential operation, no relay admission, no Start. */
test('reconcile-agent CLI: interrupted added slots recover exactly; wrong key, foreign genesis and ambiguity refuse inertly', { timeout: 180000 }, async () => {
  const root = mkdtempSync(join(tmpdir(), 'beehive-pairing-cli-reconcile-'));
  const credentialsFile = join(root, 'synthetic-credentials.json');
  const credentials = isolatedFileCredentials(credentialsFile);
  const owner = newKey(), ownerPublic = publicKey(owner);
  const host = 'reconcile-host';
  async function cli(args: string[], responses: [string, string][] = [], success = true) {
    const env = { PATH: '/usr/bin:/bin', HOME: root, BEEHIVE_TEST_CREDENTIAL_FILE: credentialsFile };
    const child = spawn(process.execPath, ['--import', resolve('test/isolated-credentials-loader.ts'), resolve('src/cli.ts'), ...args], { env, stdio: ['pipe', 'pipe', 'pipe'] });
    let output = ''; child.stdout.on('data', b => output += b); child.stderr.on('data', b => output += b);
    const exited = new Promise(resolve => child.on('close', resolve));
    try {
      for (const [prompt, response] of responses) {
        let found = false;
        for (let i = 0; i < 300; i++) { if (output.includes(prompt)) { found = true; break; } if (child.exitCode !== null) break; await delay(10); }
        assert.ok(found, `Missing CLI prompt ${prompt}`);
        child.stdin.write(`${response}\n`);
      }
      const code = await exited;
      assert.equal(code === 0, success, `CLI outcome for ${args.join(' ')}: ${output}`);
      for (const [prompt, secret] of responses) if (/^[0-9a-f]{64}$/.test(secret)) assert.ok(!output.includes(secret), `hidden input ${prompt} leaked`);
      return output;
    } finally { if (child.exitCode === null) child.kill('SIGKILL'); await exited; }
  }
  const injected = (afterStore: boolean) => ({ ...credentials, create(reference: Parameters<typeof credentials.create>[0], secret: Parameters<typeof credentials.create>[1]) { if (afterStore) credentials.create(reference, secret); throw Error('injected store boundary'); } });
  try {
    const dir = join(root, 'host');
    const binding = { mode: 'fixture', runner: process.execPath, args: [], workspace: root };
    const first = newKey(), firstKey = publicKey(first);
    const setup = validateSetup({ ...binding, host, ownerPublic });
    provisionCredentialSlot(dir, setup, first, createGenesis(ownerPublic, firstKey, host), credentials);
    const firstJournal = readFileSync(join(dir, 'agents', firstKey, 'journal.json'), 'utf8');
    const manifestBytes = () => readFileSync(join(dir, 'setup.json'), 'utf8');
    const journalOf = (agent: string) => readFileSync(join(dir, 'agents', agent, 'journal.json'), 'utf8');
    const agentsOf = () => JSON.parse(manifestBytes()).agents as Record<string, { setup: string }>;
    // Zero interrupted additions refuses before any prompt.
    assert.ok((await cli(['reconcile-agent', dir], [], false)).includes('No interrupted added-slot journal'));
    // Interrupted standby addition (store-then-throw) retains an orphan journal and key.
    const second = newKey(), secondKey = publicKey(second);
    const standbyRoot = createGenesis(ownerPublic, secondKey, 'retained-source');
    assert.throws(() => addCredentialSlot(dir, second, standbyRoot, 'default', undefined, injected(true)), /injected/);
    const secondJournal = journalOf(secondKey), manifestBefore = manifestBytes();
    assert.equal(orphanCredentialSlots(dir).orphans.length, 1);
    // Matching recovery: preview names binding and fingerprint before any input.
    const recovered = await cli(['reconcile-agent', dir], [['Reconcile this interrupted added slot', 'yes'], ['Agent private key', second]]);
    assert.ok(recovered.includes('binding default') && recovered.includes(`owner ${ownerPublic}`), 'preview must expose the derived binding');
    assert.equal(journalOf(secondKey), secondJournal, 'recovery preserves the orphan journal byte-identically');
    assert.equal(agentsOf()[secondKey]!.setup, 'default');
    assert.equal(journalOf(firstKey), firstJournal, 'first journal preserved');
    // The activated identity is no longer an orphan; recovery refuses it.
    assert.ok((await cli(['reconcile-agent', dir, secondKey], [], false)).includes('No interrupted added-slot journal for this agent public key'));
    // Declining leaves all state inert.
    const third = newKey(), thirdKey = publicKey(third);
    assert.throws(() => addCredentialSlot(dir, third, createGenesis(ownerPublic, thirdKey, host), 'default', undefined, injected(false)), /injected/);
    const thirdJournal = journalOf(thirdKey);
    await cli(['reconcile-agent', dir, thirdKey], [['Reconcile this interrupted added slot', 'no']]);
    const afterDecline = manifestBytes();
    assert.ok(afterDecline.includes(secondKey) && !afterDecline.includes(thirdKey), 'declined reconcile must not activate the slot');
    // A wrong key refuses inertly against the supplied identity.
    assert.ok((await cli(['reconcile-agent', dir, thirdKey], [['Reconcile this interrupted added slot', 'yes'], ['Agent private key', newKey()]], false)).includes('Genesis ownership mismatch'));
    assert.equal(manifestBytes(), afterDecline);
    assert.equal(journalOf(thirdKey), thirdJournal);
    // A provided public genesis must match the retained journal contents.
    const foreignFile = join(root, 'foreign-genesis.json');
    writePrivate(foreignFile, createGenesis(ownerPublic, thirdKey, 'another-host'));
    assert.ok((await cli(['reconcile-agent', dir, thirdKey, foreignFile], [], false)).includes('Provided public genesis differs'));
    assert.equal(manifestBytes(), afterDecline);
    // Ambiguous orphan selection without an agent public key refuses.
    const fourth = newKey(), fourthKey = publicKey(fourth);
    const fourthRoot = createGenesis(ownerPublic, fourthKey, 'retained-source');
    assert.throws(() => addCredentialSlot(dir, fourth, fourthRoot, 'default', undefined, injected(false)), /injected/);
    const fourthJournal = journalOf(fourthKey);
    assert.ok((await cli(['reconcile-agent', dir], [], false)).includes('Specify the agent public key for a multi-slot host'));
    // Standby recovery with the optional matching public genesis file.
    const genesisFile = join(root, 'standby-genesis.json');
    writePrivate(genesisFile, { initialHost: fourthRoot.initialHost, agent: fourthRoot.agent, owner: fourthRoot.owner, id: fourthRoot.id, v: 1 as const });
    await cli(['reconcile-agent', dir, fourthKey, genesisFile], [['Reconcile this interrupted added slot', 'yes'], ['Agent private key', fourth]]);
    assert.equal(journalOf(fourthKey), fourthJournal);
    assert.equal(agentsOf()[fourthKey]!.setup, 'default');
    assert.equal((readPrivate(join(dir, 'agents', fourthKey, 'journal.json')) as { assignment: { assignedHost: string } }).assignment.assignedHost, 'retained-source');
    // A non-default immutable binding is derived from the orphan journal, never defaulted.
    // (Binding inventory extension is the separate v3 wizard scope; only recovery semantics are exercised here.)
    const manifest = readPrivate(join(dir, 'setup.json')) as { setups: Record<string, { runner: string; args: string[]; workspace: string; mode: string }> };
    manifest.setups.second = { ...manifest.setups.default!, workspace: join(root, 'w2') };
    writePrivate(join(dir, 'setup.json'), manifest);
    const fifth = newKey(), fifthKey = publicKey(fifth);
    assert.throws(() => addCredentialSlot(dir, fifth, createGenesis(ownerPublic, fifthKey, host), 'second', undefined, injected(false)), /injected/);
    const fifthJournal = journalOf(fifthKey);
    const chosen = await cli(['reconcile-agent', dir, fifthKey], [['Reconcile this interrupted added slot', 'yes'], ['Agent private key', fifth]]);
    assert.ok(chosen.includes('binding second'), 'preview must expose the journal-derived binding');
    assert.equal(agentsOf()[fifthKey]!.setup, 'second');
    assert.equal(journalOf(fifthKey), fifthJournal);
    // Ambiguous binding definitions require an explicit choice; a wrong choice refuses.
    const extended = readPrivate(join(dir, 'setup.json')) as typeof manifest;
    extended.setups.twin = { ...extended.setups.second!, args: [join(root, 'twin-arg')] };
    writePrivate(join(dir, 'setup.json'), extended);
    const sixth = newKey(), sixthKey = publicKey(sixth);
    assert.throws(() => addCredentialSlot(dir, sixth, createGenesis(ownerPublic, sixthKey, host), 'second', undefined, injected(false)), /injected/);
    const sixthJournal = journalOf(sixthKey);
    assert.ok((await cli(['reconcile-agent', dir, sixthKey], [['Enter the binding id', 'default']], false)).includes('Chosen binding does not match the retained journal'));
    assert.equal(agentsOf()[sixthKey], undefined, 'wrong binding choice must not activate');
    await cli(['reconcile-agent', dir, sixthKey], [['Enter the binding id', 'twin'], ['Reconcile this interrupted added slot', 'yes'], ['Agent private key', sixth]]);
    assert.equal(agentsOf()[sixthKey]!.setup, 'twin');
    assert.equal(journalOf(sixthKey), sixthJournal);
    // Final state: five activated public slots, one retained orphan (third), no secrets anywhere.
    assert.equal(Object.keys(agentsOf()).length, 5);
    const remaining = orphanCredentialSlots(dir).orphans;
    assert.equal(remaining.length, 1);
    assert.equal(remaining[0]!.agent, thirdKey);
    const loaded = installationSlots(dir, credentials);
    assert.equal(loaded.length, 5);
    const finalManifest = manifestBytes();
    for (const secret of [owner, first, second, third, fourth, fifth, sixth]) assert.ok(!finalManifest.includes(secret), 'manifest must not persist secrets');
    assert.equal(journalOf(firstKey), firstJournal, 'original journal byte-identical throughout');
  } finally { rmSync(root, { recursive: true, force: true }); }
});
