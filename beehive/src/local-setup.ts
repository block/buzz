import { createInterface } from 'node:readline/promises';
import { stdin, stdout } from 'node:process';
import { realpathSync } from 'node:fs';
import { installationSlots, addHarnessBinding, addSlot, importSlotKey } from './slots.ts';
import { bindingFingerprint } from './host.ts';
import { newKey, publicKey, text } from './protocol.ts';
import { readPrivate } from './storage.ts';
import { createGenesis, validateGenesis } from './assignment.ts';
import { readAgentSecret } from './key-input.ts';

/** Small offline wizard: one confirmed local mutation, never remote selection or login.
 * Binding revisions are new IDs; existing definitions/history are not edited/deleted.
 */
export async function localSetup(directory: string): Promise<void> {
  const entries = installationSlots(directory), first = entries[0]!;
  const ui = createInterface({ input: stdin, output: stdout });
  try {
    console.log(`Local installation ${first.setup.host}. Stop the host before committing changes. No login or process launch.`);
    for (const [id, setup] of Object.entries(first.bindings)) console.log(`Binding ${id}: ${setup.mode} | ${bindingFingerprint(setup)}`);
    for (const entry of entries) console.log(`Agent ${entry.agent}: ${entry.keyPresent ? 'reuse existing key' : 'public-only; exact local key restoration required'} | binding ${entry.setupId}`);
    const action = await ui.question('Local action [reuse / new-agent / restore-key / import-standby / add-binding / add-goose / cancel]: ');
    if (action === 'cancel') return;
    if (action === 'reuse' || action === 'restore-key') {
      const agent = await ui.question('Existing agent public key: ');
      const entry = entries.find(e => e.agent === agent);
      if (!entry) throw Error('Unknown retained agent; cannot invent assignment');
      if (action === 'reuse') {
        if (!entry.keyPresent) throw Error('Local key missing; use restore-key explicitly');
        console.log(`Reuse ${agent}; no local mutation. In remote TUI select this host/agent, choose binding/configuration, Save then explicitly Restart. Assignment still gates Start.`);
        return;
      }
      if (entry.keyPresent) throw Error('Local key already present; choose reuse');
      if (await ui.question('Restore exact retained key only, preserving assignment/history? [yes/no]: ') !== 'yes') return;
      ui.close(); // Secret reader owns terminal echo; never overlap readline owners.
      importSlotKey(directory, agent, await readAgentSecret());
      console.log(`Restored local key for ${agent}; no assignment or history reset, no Start permission added.`);
      return;
    }
    if (!['new-agent', 'add-binding', 'add-goose', 'import-standby'].includes(action)) throw Error('Unsupported local action');
    const id = await ui.question('Existing binding ID to reuse: ');
    if (!Object.hasOwn(first.bindings, id)) throw Error('Unknown local binding');
    const setup = first.bindings[id]!, fingerprint = bindingFingerprint(setup);
    if (action === 'import-standby') {
      const genesis = validateGenesis(readPrivate(realpathSync(text(await ui.question('Local public genesis file (no key): ')))));
      if (genesis.owner !== publicKey(setup.ownerSecret) || genesis.initialHost === setup.host) throw Error('Standby import requires matching owner and another initial host');
      if (entries.some(e => e.agent === genesis.agent)) throw Error('Retained identity exists; choose reuse or explicit restore-key');
      if (await ui.question(`Import exact ${genesis.agent} as standby on ${id}; Start remains assigned to ${genesis.initialHost}? [yes/no]: `) !== 'yes') return;
      ui.close();
      const secret = await readAgentSecret();
      if (publicKey(secret) !== genesis.agent) throw Error('Imported key does not match public genesis');
      addSlot(directory, secret, genesis, id, fingerprint);
      console.log(`Standby ${genesis.agent} saved using ${id}; key possession grants no Start. No key/session/workspace transfer.`);
      return;
    }
    if (action === 'add-goose') {
      const nextId = text(await ui.question('NEW immutable Goose binding ID: '));
      const runner = realpathSync(text(await ui.question('Absolute installed Goose executable (runs acp): ')));
      const workspace = realpathSync(text(await ui.question('Allowed workspace (absolute directory): ')));
      const serviceHome = realpathSync(text(await ui.question('Existing dedicated service HOME (not Desktop HOME): ')));
      const gooseProvider = text(await ui.question('Locally configured Goose provider ID: '));
      const gooseModels = text(await ui.question('Operator-approved compatible exact model IDs (comma-separated): ')).split(',').map(m => m.trim());
      console.log(`Goose owns provider configuration and credentials in its service-user context, HOME=${serviceHome}, ~/.config/goose/config.yaml. Configure Goose locally as the same OS service user (current uid ${process.getuid?.() ?? 'unknown'}). No generic API-key field, Buzz Agent OAuth reuse, login or authentication probe. Executable availability is not authentication; approved models are not a provider catalog. Start requires native exact model evidence; settings changes require fresh Restart.`);
      if (await ui.question('Save NEW Goose binding only (no identity/selection/restart)? [yes/no]: ') !== 'yes') return;
      addHarnessBinding(directory, nextId, { mode: 'goose', runner, args: ['acp'], workspace, allowedWorkspaces: [workspace], serviceHome, configDirectory: serviceHome, gooseProvider, gooseModels, ...(setup.conversation ? { conversation: setup.conversation } : {}) }, { id, fingerprint });
      console.log(`Goose binding ${nextId} saved. Reuse an existing identity remotely or reopen local-setup for new/import-standby; common owner/relay/authority unchanged.`);
      return;
    }
    if (action === 'new-agent') {
      if (await ui.question(`Create NEW independent identity using ${id}? [yes/no]: `) !== 'yes') return;
      const secret = newKey();
      addSlot(directory, secret, createGenesis(publicKey(setup.ownerSecret), publicKey(secret), setup.host), id, fingerprint);
      console.log(`Added ${publicKey(secret)} using ${id}; configure and start remotely. Existing keys/history unchanged.`);
      return;
    }
    const nextId = text(await ui.question('NEW immutable binding ID: '));
    const runner = realpathSync(text(await ui.question('Absolute compatible executable: ')));
    const workspace = realpathSync(text(await ui.question('Allowed workspace (absolute directory): ')));
    const args = setup.mode === 'fixture' ? [realpathSync(text(await ui.question('Absolute fixture TypeScript script: ')))] : setup.args;
    const { host: _host, ownerSecret: _owner, agentSecret: _agent, ...harness } = setup;
    console.log(`Create ${nextId} from ${id}; same ${setup.mode} contract and local service auth context. This does not add a provider or change conversation relay/authority. No authentication tested. Old binding remains immutable; no edit/remove supported.`);
    if (await ui.question('Save NEW binding only (no selection, key change or restart)? [yes/no]: ') !== 'yes') return;
    addHarnessBinding(directory, nextId, { ...harness, runner, args, workspace, allowedWorkspaces: [workspace] }, { id, fingerprint });
    console.log(`Binding ${nextId} saved. Reopen local-setup to reuse it for a new identity, or select it in the remote TUI for an existing identity. Save does not Restart.`);
  } finally { ui.close(); }
}
