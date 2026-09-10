import { createInterface } from 'node:readline/promises';
import { stdin, stdout } from 'node:process';
import { hostname, homedir } from 'node:os';
import { join } from 'node:path';
import { existsSync } from 'node:fs';
import { decode } from 'nostr-tools/nip19';
import { bootstrapHostIdentity, readHostIdentityPublic, readHostIdentityAsync } from './host-identity.ts';
import { systemCredentials } from './credential-store.ts';
import { credentialHelperReader } from './credential-helper.ts';
import { membershipSigner } from './relay-admission.ts';
import { claimRelayMembershipInvite, relayHttpOrigin, verifyDirectMembershipEvidence } from './direct-membership.ts';

const quote = (s: string) => `'${s.replaceAll("'", "'\"'\"'")}'`;
/** Public owner input only; nsec and other NIP-19 payloads are never accepted. */
export function ownerPublicInput(input: string): string {
  const value = input.trim();
  if (/^[0-9a-fA-F]{64}$/.test(value)) return value.toLowerCase();
  try { const decoded = decode(value); if (decoded.type === 'npub') return decoded.data; } catch { /* actionable public-input error below */ }
  throw Error('Owner PUBLIC npub or 64-character hex key required; never enter a private key');
}
/** Existing Buzz invite formats, scoped to the retained relay; never retarget. */
export function hostInviteCode(input: string, relay: string): string {
  const value = input.trim();
  let code = value, origin: string | undefined;
  if (value.includes('://')) {
    const url = new URL(value);
    if (url.username || url.password || url.hash) throw Error('Invalid invite link');
    if (url.protocol === 'buzz:' && url.host === 'join') {
      origin = url.searchParams.get('relay') ?? '';
      code = url.searchParams.get('code') ?? '';
    } else if (url.protocol === 'https:' || url.protocol === 'http:') {
      const match = url.pathname.match(/^\/invite\/([^/]+)\/?$/);
      if (!match) throw Error('Expected community /invite/<code> link');
      origin = url.origin; code = decodeURIComponent(match[1]!);
    } else throw Error('Expected community invite link or issued code');
    if (relayHttpOrigin(origin).href !== relayHttpOrigin(relay).href) throw Error('Invite is for another relay; retained configuration unchanged');
  } else if (value.includes('/')) throw Error('Expected community invite link or issued code');
  if (!code || code.length > 1024 || /[\x00-\x20\x7f]/.test(code)) throw Error('Invalid issued invite code');
  return code;
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
      relayHttpOrigin(relay);
      if (!/^(wss|ws):\/\//.test(relay)) throw Error('Management relay requires wss:// (ws://127.0.0.1 for fixtures)');
      console.log(`Computer name: ${hostname()}. Host folder: ${directory}`);
      if (await ui.question('Create host identity in Beehive OS credentials? Owner present; OS access may prompt. [yes/no]: ') !== 'yes') return;
      controller.signal.throwIfAborted();
      retained = bootstrapHostIdentity(directory, hostname().slice(0, 128), owner, relay);
    }
    console.log(`Configured host: ${retained.pairing.host}\nOwner: ${retained.pairing.owner}\nRelay: ${retained.pairing.relay}\nIdentity, owner, relay and approval history retained. Not serving. Resume: beehive setup${command}`);
    if (await ui.question('Check/join community using this host key? Owner present; OS credential read may prompt. [yes/no]: ') !== 'yes') return;
    const identity = await readHostIdentityAsync(directory, { ...systemCredentials, readAsync: credentialHelperReader({ operatorApproved: true }) }, controller.signal);
    const signer = membershipSigner(identity.secret), relay = identity.pairing.relay;
    let evidence = await verifyDirectMembershipEvidence({ relay, signer, signal: controller.signal });
    controller.signal.throwIfAborted();
    if (evidence.status === 'denied') {
      console.log('Community membership required for this host key. Use an operator-issued invite, or ask an administrator to admit the displayed host public key. Policy acceptance may require the community’s normal client/admin flow; this wizard cannot fabricate a policy receipt.');
      const supplied = await ui.question('Issued invite code or link (Enter to cancel and resume later): ');
      if (!supplied.trim()) return;
      const inviteCode = hostInviteCode(supplied, relay);
      if (await ui.question('Claim this issued invite for this host on the retained relay? [yes/no]: ') !== 'yes') return;
      await claimRelayMembershipInvite({ relay, signer, inviteCode, signal: controller.signal });
      evidence = await verifyDirectMembershipEvidence({ relay, signer, signal: controller.signal });
    }
    controller.signal.throwIfAborted();
    if (evidence.status !== 'live-verified-member') throw Error(`Community membership pending (${evidence.status}); require a membership-enforced relay and fresh live check. Retry setup after network/operator action`);
    console.log(`Community membership verified now; host configured, NOT serving. Start foreground deliberately: beehive host${command} --owner-present\nStartup rechecks membership and runtime support. Ctrl-C stops the host. No agent starts automatically.\nOn the trusted owner computer: beehive tui discover ${quote(relay)} ~/.beehive/owner\nOwner signer stays there. Use hosts to discover private availability; availability is not agent authorization. Provider configuration and explicit per-agent grants/placement remain separate.`);
  } catch (error) {
    console.error(`Setup incomplete; no host started. Preserve the host folder and credentials; no automatic reset. Resume: beehive setup${command}`);
    throw error;
  } finally { ui.close(); process.off('SIGINT', stop); process.off('SIGTERM', stop); }
}
