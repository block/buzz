import { createInterface } from 'node:readline/promises';
import { stdin, stdout } from 'node:process';
import { join } from 'node:path';
import { hostname } from 'node:os';
import { existsSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { bootstrapHostIdentity, enrollHostIdentity, readHostIdentityPublic } from './host-identity.ts';
import { hostPairing, pairingFingerprint, registerHost, verifyHostRegistration } from './host-registration.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { readAgentSecret } from './key-input.ts';
import { text } from './protocol.ts';
import { setupPath } from './setup-path.ts';

const quote = (s: string) => `'${s.replaceAll("'", "'\"'\"'")}'`;
/** Standalone offline enrollment UI. File exchange avoids circular relay bootstrap. */
export async function enrollmentInput(directory: string): Promise<void> {
  const retained = existsSync(join(directory, 'host-identity.json')) ? readHostIdentityPublic(directory) : undefined;
  const exchange = join(directory, 'exchange');
  const ui = createInterface({ input: stdin, output: stdout });
  try {
    console.log('Prepare this computer → owner approval → import. No relay connection, agent authorization, or Start. Approve on the trusted owner computer, never enter the owner private key on the host.');
    if (retained) console.log(`Existing computer identity retained: ${retained.pairing.label}. Enter export to continue without creating or reading a key, or import when approval returns.`);
    console.log('create/export save requests automatically; export-to/approve-to let you choose a new output path.');
    const action = (await ui.question(`Enrollment [${retained ? 'export (Enter) / import / export-to / cancel' : 'create / approve / approve-to / cancel'}]: `)) || (retained ? 'export' : '');
    if (action === 'cancel') return;
    if (action === 'create' || action === 'export' || action === 'export-to') {
      if (action === 'create' && retained) throw Error('Host identity already exists. Rerun setup and choose export or import; do not reset it.');
      const identity = action === 'create' ? bootstrapHostIdentity(directory,
        (await ui.question(`Computer name [${hostname()}]: `)) || hostname(),
        text(await ui.question('Owner PUBLIC key (hex, from your existing owner identity; never the private key): ')),
        text(await ui.question('Management relay URL (wss://… from your community operator; membership is separate): '))) : readHostIdentityPublic(directory);
      const fingerprint = pairingFingerprint(identity.pairing);
      console.log(`Pairing fingerprint: ${fingerprint}`);
      const file = action === 'export-to' ? setupPath(text(await ui.question('New private request output path (~ supported): '))) : join(exchange, `request-${fingerprint}-${randomUUID()}.json`);
      writePrivate(file, identity.pairing, true);
      console.log(`Request saved: ${file}\nTransfer this file privately to the owner computer (contains public keys and private infrastructure metadata, no secrets). There run beehive legacy-enrollment ~/.beehive/owner-exchange, choose approve and supply the transferred file. Compare the full fingerprint above. Re-export is safe; older files are never replaced.`);
    } else if (action === 'approve' || action === 'approve-to') {
      if (retained) throw Error('This folder belongs to a host. Approve on the trusted owner computer using beehive legacy-enrollment ~/.beehive/owner-exchange; no owner key requested here.');
      const request = hostPairing(readPrivate(setupPath(text(await ui.question('Private pairing file (transferred from host, ~ supported): ')))));
      console.log(`Host: ${request.host}\nLabel: ${request.label}\nOwner: ${request.owner}\nRelay: ${request.relay}\nFingerprint: ${pairingFingerprint(request)}`);
      if (await ui.question('Compare full fingerprint with host. Register ONLY this infrastructure (not agents or relay delegation)? [yes/no]: ') !== 'yes') return;
      const duration = await ui.question('Approval validity (e.g. 7d for seven days, or future Unix seconds; choose how long to trust this host): ');
      const expires = /^\d+d$/.test(duration) ? Math.floor(Date.now() / 1000) + Number(duration.slice(0, -1)) * 86400 : Number(duration);
      if (!Number.isSafeInteger(expires) || expires <= Math.floor(Date.now() / 1000)) throw Error('Expiration must be in the future; use e.g. 7d or future Unix seconds');
      console.log(`Approval expires: ${new Date(expires * 1000).toISOString()}`);
      const file = action === 'approve-to' ? setupPath(text(await ui.question('New private approval output path (~ supported): '))) : join(exchange, `approval-${pairingFingerprint(request)}-${randomUUID()}.json`);
      ui.close();
      const secret = await readAgentSecret('Owner');
      writePrivate(file, registerHost(request, secret, expires), true);
      console.log(`Approval saved: ${file}\nOwner secret not persisted. Transfer privately back to host, then run beehive legacy-enrollment and choose import. You may put it at the host's displayed default approval path.\nOn this owner computer next: beehive catalog ${quote(file)}\nRelay admission remains pending.`);
    } else if (action === 'import') {
      if (!retained) throw Error('No host identity here. Run setup on the original host folder; do not create a replacement identity.');
      const expected = join(exchange, `approval-${pairingFingerprint(retained.pairing)}.json`);
      console.log(`Transfer the owner approval here for an Enter default: ${expected}. Or enter its actual transferred path. Filename is not authorization; exact request and owner signature are checked.`);
      const supplied = await ui.question(`Private approval file (~ supported${existsSync(expected) ? ', Enter for displayed path' : '; transferred from owner'}): `);
      if (!supplied && !existsSync(expected)) throw Error('No approval at the displayed path. Transfer the owner-signed approval, then rerun setup and choose import.');
      const approval = readPrivate(supplied ? setupPath(supplied) : expected);
      verifyHostRegistration(approval, retained.pairing);
      enrollHostIdentity(directory, approval);
      console.log(`Host registered offline. Pending/no network: relay admission remains pending. Ask your relay operator to enroll the host and owner as direct members. No agent authorized or started.\nNext: prepare binding and owner-approved agent genesis as in FIRST_DEMO.md, then beehive provision-agent ${quote(directory)} <binding-file> <genesis-file>. Provider login is separate; beehive auth-info ${quote(directory)} prints its context. Then beehive host ${quote(directory)} --owner-present.`);
    } else throw Error('Choose create to prepare a new computer, export to resume, approve on the owner computer, or import a returned approval');
  } finally { ui.close(); }
}
