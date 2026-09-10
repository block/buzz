import { createInterface } from 'node:readline/promises';
import { stdin, stdout } from 'node:process';
import { resolve } from 'node:path';
import { bootstrapHostIdentity, enrollHostIdentity, readHostIdentity } from './host-identity.ts';
import { hostPairing, pairingFingerprint, registerHost } from './host-registration.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { readAgentSecret } from './key-input.ts';
import { text } from './protocol.ts';

/** Standalone offline enrollment UI. File exchange avoids circular relay bootstrap. */
export async function enrollmentInput(directory: string): Promise<void> {
  const ui = createInterface({ input: stdin, output: stdout });
  try {
    console.log('Private infrastructure enrollment. No relay connection, agent authority, or runtime launch. Approval must run in your owner context, not on an untrusted host.');
    const action = await ui.question('Enrollment [create / export / approve / import / cancel]: ');
    if (action === 'cancel') return;
    if (action === 'create' || action === 'export') {
      const identity = action === 'create' ? bootstrapHostIdentity(directory, text(await ui.question('Host label: ')), text(await ui.question('Owner PUBLIC key (hex): ')), text(await ui.question('Relay URL: '))) : readHostIdentity(directory);
      console.log(`Pairing fingerprint: ${pairingFingerprint(identity.pairing)}`);
      const file = resolve(text(await ui.question('NEW private pairing file: ')));
      writePrivate(file, identity.pairing, true);
      console.log('Pairing exported. Transfer privately to your standalone owner context. Pending/no network. Export can be repeated if interrupted.');
    } else if (action === 'approve') {
      const request = hostPairing(readPrivate(resolve(text(await ui.question('Private pairing file: ')))));
      console.log(`Host: ${request.host}\nLabel: ${request.label}\nOwner: ${request.owner}\nRelay: ${request.relay}\nFingerprint: ${pairingFingerprint(request)}`);
      if (await ui.question('Compare full fingerprint with host. Register ONLY this infrastructure (not agents or relay delegation)? [yes/no]: ') !== 'yes') return;
      const expires = Number(await ui.question('Registration expiration (Unix seconds): '));
      if (!Number.isSafeInteger(expires) || expires <= Math.floor(Date.now() / 1000)) throw Error('Expiration must be in the future');
      const file = resolve(text(await ui.question('NEW private approval file: ')));
      ui.close();
      const secret = await readAgentSecret('Owner');
      writePrivate(file, registerHost(request, secret, expires), true);
      console.log('Registration signed. Owner secret not persisted. Transfer approval privately back to host. Relay admission remains pending.');
    } else if (action === 'import') {
      enrollHostIdentity(directory, readPrivate(resolve(text(await ui.question('Private approval file: ')))));
      console.log('Host registered offline. Pending/no network: relay admission remains pending. Host startup requires an already-enrolled direct relay member and a fresh live row check. No agent authorized or started.');
    } else throw Error('Unknown enrollment action');
  } finally { ui.close(); }
}
