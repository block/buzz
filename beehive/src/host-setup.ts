import { createInterface } from 'node:readline/promises';
import { stdin, stdout } from 'node:process';
import { hostname, homedir } from 'node:os';
import { join } from 'node:path';
import { existsSync } from 'node:fs';
import { decode } from 'nostr-tools/nip19';
import { bootstrapHostIdentity, readHostIdentityPublic } from './host-identity.ts';

const quote = (s: string) => `'${s.replaceAll("'", "'\"'\"'")}'`;
/** Public owner input only; nsec and other NIP-19 payloads are never accepted. */
export function ownerPublicInput(input: string): string {
  const value = input.trim();
  if (/^[0-9a-fA-F]{64}$/.test(value)) return value.toLowerCase();
  try { const decoded = decode(value); if (decoded.type === 'npub') return decoded.data; } catch { /* actionable public-input error below */ }
  throw Error('Owner PUBLIC npub or 64-character hex key required; never enter a private key');
}
/** Configure/resume infrastructure only. No owner secret, approval, agent grant or Start. */
export async function hostSetup(directory: string): Promise<void> {
  let retained = existsSync(join(directory, 'host-identity.json')) ? readHostIdentityPublic(directory) : undefined;
  const ui = createInterface({ input: stdin, output: stdout });
  const controller = new AbortController();
  const stop = () => { controller.abort(Error('Setup cancelled; retained configuration unchanged')); ui.close(); };
  process.on('SIGINT', stop); process.on('SIGTERM', stop); ui.on('SIGINT', stop);
  const command = directory === join(homedir(), '.beehive', 'host') ? '' : ` ${quote(directory)}`;
  try {
    console.log('Configure this host with OWNER NPUB + RELAY. No infrastructure approval or file exchange. No agent, provider credentials or grants created.');
    if (!retained) {
      const owner = ownerPublicInput(await ui.question('Owner PUBLIC npub (hex also accepted): '));
      const relay = (await ui.question('Management relay URL (wss://…): ')).trim();
      if (!/^(wss|ws):\/\//.test(relay)) throw Error('Management relay requires wss:// (ws://127.0.0.1 for fixtures)');
      console.log(`Computer name: ${hostname()}. Host folder: ${directory}`);
      if (await ui.question('Create host identity in Beehive OS credentials? Owner present; OS access may prompt. [yes/no]: ') !== 'yes') return;
      controller.signal.throwIfAborted();
      retained = bootstrapHostIdentity(directory, hostname().slice(0, 128), owner, relay);
    }
    console.log(`Configured host: ${retained.pairing.host}\nOwner: ${retained.pairing.owner}\nRelay: ${retained.pairing.relay}\nIdentity, owner, relay and approval history retained. Not serving. Resume: beehive setup${command}`);
    const relay = retained.pairing.relay;
    controller.signal.throwIfAborted();
    console.log(`Host configured, NOT serving. Start foreground deliberately: beehive host${command} --owner-present\nStartup uses private NIP42/NIP59 transport and waits for accepted availability publication. Ctrl-C stops the host. No agent starts automatically.\nOn the trusted owner computer: beehive tui discover ${quote(relay)} ~/.beehive/owner\nOwner signer stays there. Use hosts to discover private availability; availability is not agent authorization. Provider configuration and explicit per-agent grants/placement remain separate.`);
  } catch (error) {
    console.error(`Setup incomplete; no host started. Preserve the host folder and credentials; no automatic reset. Resume: beehive setup${command}`);
    throw error;
  } finally { ui.close(); process.off('SIGINT', stop); process.off('SIGTERM', stop); }
}
