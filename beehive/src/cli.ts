import { readAgentSecret } from './key-input.ts';
import { Profiles, profile, profileRevision, type Profile } from './profiles.ts';
import { createInterface } from 'node:readline/promises';
import { stdin, stdout } from 'node:process';
import { existsSync, mkdirSync, realpathSync, rmdirSync } from 'node:fs';
import { resolve, join, dirname } from 'node:path';
import { relay } from './relay.ts';
import { host, validateSetup, provision, migrateAssignment } from './host.ts';
import { migrateSlots, addSlot, installationSlots, saveDefaultHarness, removeSlotKey, importSlotKey } from './slots.ts';
import { managementClient } from './intents.ts';
import { message, newKey, publicKey, object, text, type Message } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { createGenesis, validateGenesis } from './assignment.ts';
import { prepareConversation } from './conversation.ts';

const operationLabel = (m: Message) => `${m.host} ${m.type}${m.type === 'move' ? ` → ${String(m.body.target)}` : ''} | agent ${m.agent} | operation ${m.id}`;
const shellQuote = (s: string) => `'${s.replaceAll("'", "'\"'\"'")}'`;
const [command, ...args] = process.argv.slice(2);
const help = `Beehive — isolated development preview (loopback relay only)
  identity <new-directory>                  Create a NEW local owner identity
  setup <new-host-directory> <identity-file> Guided local harness + key setup
  migrate-slots <host-directory>            Explicit stopped upgrade, preserves journal
  add-agent <host-directory>                New identity using shared local harness
  remove-agent-key <host-directory> [agent-public-key] Remove ONE local key copy (public slot retained)
  import-agent-key <host-directory> <agent-public-key> Restore retained identity via hidden local entry
  assignment-export <host-directory> <new-file> Export public pinned genesis locally
  migrate-assignment <host-directory>       Explicit stopped legacy enrollment
  auth-info <host-directory>                Print exact local sign-in context (no login)
  conversation-setup <host-directory>       Attach external buzz-acp to EXISTING identity (Start gated)
  relay <port> <owner-public-key> <log-file> Dedicated ciphertext relay
  host <host-directory> <ws://127.0.0.1:port> Persistent foreground host
  tui <identity-file> <ws://127.0.0.1:port>   Relay-connected terminal UI
setup/add-agent accept optional <local-key-file> <public-genesis-file> for standby import.
assignment-export accepts agent public key after filename when several slots exist.
Move is fixture-only experimental; containment acceptance remains gated. No provider login RPC or production relay support.`;
async function main() {
  if (command === 'identity') {
    const dir = resolve(text(args[0]));
    mkdirSync(dir,{ mode: 0o700 });
    const secret = newKey();
    writePrivate(join(dir,'identity.json'),{ secret });
    console.log(`Owner public key: ${publicKey(secret)}\nIdentity file: ${join(dir,'identity.json')}`);
  } else if (command === 'setup') {
    const dir = resolve(text(args[0]));
    if (existsSync(dir)) throw Error('Use a new host directory; setup cannot reset authority');
    const secret = text(object(readPrivate(resolve(text(args[1])))).secret);
    const ui = createInterface({ input: stdin, output: stdout });
    try {
      const name = text(await ui.question('Host name: '));
      const mode = await ui.question('Setup [1 deterministic fixture / 2 Buzz Agent + Databricks v2]: ');
      if (!['1','2'].includes(mode)) throw Error('Choose 1 or 2');
      const runner = realpathSync(text(await ui.question(mode === '1' ? 'Absolute fixture runner executable: ' : 'Absolute installed buzz-agent executable: ')));
      const workspace = realpathSync(text(await ui.question('Allowed workspace (absolute directory): ')));
      const additionalWorkspace = await ui.question('Additional allowed workspace (blank for none): ');
      const allowedWorkspaces = additionalWorkspace ? [workspace, realpathSync(text(additionalWorkspace))] : [workspace];
      const extra = mode === '1' ? text(await ui.question('Absolute fixture TypeScript script: ')) : '';
      const databricksHost = mode === '2' ? text(await ui.question('Databricks workspace HTTPS URL: ')) : undefined;
      if (databricksHost) { const url = new URL(databricksHost); if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash || url.pathname !== '/') throw Error('Databricks workspace must be an HTTPS origin without credentials'); }
      console.log(mode === '1' ? 'Fixture setup: no provider/login; trusted local executable, no agent credentials passed.' : `Buzz Agent Databricks v2 owns browser OAuth and refresh. Intended host/service credential context: uid ${process.getuid?.() ?? 'unknown'}, HOME=${shellQuote(join(dir, 'service-home'))}, BUZZ_AGENT_CONFIG_DIR=${shellQuote(join(dir, 'agent-config'))}, DATABRICKS_HOST=${shellQuote(databricksHost!)}. Run ${shellQuote(runner)} auth databricks in exactly that context as the service user; no login initiated. Missing executable: install buzz-agent locally. Missing auth: use auth-info after setup.`);
      const importing = args.length > 2;
      const agentSecret = importing ? text(object(readPrivate(resolve(text(args[2])))).secret) : newKey();
      const genesis = importing ? validateGenesis(readPrivate(resolve(text(args[3])))) : createGenesis(publicKey(secret), publicKey(agentSecret), name);
      if (importing && genesis.initialHost === name) throw Error('Import cannot create a second initial authority installation');
      if ((await ui.question(importing ? `Provision matching key as standby only; assignment remains ${genesis.initialHost}? [yes/no]: ` : 'Create a NEW agent identity assigned exclusively to this host? [yes/no]: ')) !== 'yes') return;
      mkdirSync(dir,{ mode: 0o700 });
      if (mode === '2') { mkdirSync(join(dir,'service-home'),{ mode: 0o700 }); mkdirSync(join(dir,'agent-config'),{ mode: 0o700 }); }
      provision(dir,{ host: name, ownerSecret: secret, agentSecret, runner, args: mode === '1' ? [resolve(extra)] : [], workspace, allowedWorkspaces, mode: mode === '1' ? 'fixture' : 'buzz-agent-databricks-v2', ...(databricksHost ? { databricksHost, serviceHome: join(dir,'service-home'), configDirectory: join(dir,'agent-config') } : {}) }, genesis);
      migrateSlots(dir);
      console.log(`Harness setup default is reusable; agent ${publicKey(agentSecret)} is independent. Host must remain stopped for local structural changes.`);
      while ((await ui.question('Add another independent NEW agent using this same harness setup? [yes/no]: ')) === 'yes') {
        const additional = newKey();
        addSlot(dir, additional, createGenesis(publicKey(secret), publicKey(additional), name));
        console.log(`Added agent ${publicKey(additional)} using harness setup default; no provider setup or key copy.`);
      }
      console.log('Local setup saved. Start the host separately; the TUI never owns its lifetime.');
      if (mode === '2') console.log(`Not authenticated. Run auth-info ${shellQuote(dir)} for the exact local host/service-user login command. Start uses an ACP greeting probe, not yet a Buzz relay conversation agent.`);
    } finally { ui.close(); }
  } else if (command === 'migrate-slots') {
    const dir = resolve(text(args[0]));
    const ui = createInterface({ input: stdin, output: stdout });
    try {
      if (await ui.question('Upgrade this stopped installation to shared agent slots, preserving keys and journal? [yes/no]: ') !== 'yes') return;
      migrateSlots(dir); console.log('Slots enabled; original assignment and lifecycle history retained.');
    } finally { ui.close(); }
  } else if (command === 'add-agent') {
    const dir = resolve(text(args[0]));
    const first = installationSlots(dir)[0]!.setup;
    const importing = args.length > 1;
    const secret = importing ? text(object(readPrivate(resolve(text(args[1])))).secret) : newKey();
    const root = importing ? validateGenesis(readPrivate(resolve(text(args[2])))) : createGenesis(publicKey(first.ownerSecret), publicKey(secret), first.host);
    if (importing && root.initialHost === first.host) throw Error('Import cannot recreate initial authority');
    const ui = createInterface({ input: stdin, output: stdout });
    try {
      if (await ui.question(importing ? `Add standby identity assigned to ${root.initialHost}, reusing host harness default? [yes/no]: ` : 'Create independent NEW agent assigned here, reusing host harness default? [yes/no]: ') !== 'yes') return;
      addSlot(dir, secret, root); console.log(`Agent ${publicKey(secret)} added; start/configure remotely after restarting the host.`);
    } finally { ui.close(); }
  } else if (command === 'remove-agent-key') {
    const dir = resolve(text(args[0]));
    if (args[1] !== undefined && !/^[0-9a-f]{64}$/.test(String(args[1]))) throw Error('Invalid agent public key');
    const entries = installationSlots(dir);
    const entry = args[1] !== undefined ? entries.find(e => e.agent === args[1]) : entries.length === 1 ? entries[0] : undefined;
    if (!entry) throw Error(args[1] === undefined ? 'Specify the agent public key for a multi-slot host' : 'Unknown agent slot on this installation');
    const ui = createInterface({ input: stdin, output: stdout });
    try {
      if ((await ui.question(`Remove this installation's local key copy for agent ${entry.agent} on host ${entry.setup.host}? The public identity, assignment, configurations and complete receipt/run history stay as a public-only slot; Start/Restart reject until an explicit local repair. Only this local secret copy is deleted - not a global cryptographic revocation, and no remote operation can restore it. [yes/no]: `)) !== 'yes') return;
      removeSlotKey(dir, entry.agent);
      console.log(`Local key copy removed for agent ${entry.agent}; public-only slot retained on ${entry.setup.host}. Assignment, journal history, configurations and receipts are unchanged; Stop/Save/history stay available. Start/Restart now reject before spawn; remote operations never recreate the secret. Sibling slots and the owner identity are unaffected. Re-provision requires explicit local reconciliation.`);
    } finally { ui.close(); }
  } else if (command === 'import-agent-key') {
    if (args.length !== 2) throw Error('Use host directory and public key only; private keys are never arguments');
    const dir = resolve(text(args[0])), agent = text(args[1]);
    if (!/^[0-9a-f]{64}$/.test(agent)) throw Error('Invalid agent public key');
    const entry = installationSlots(dir).find(e => e.agent === agent);
    if (!entry) throw Error('Unknown retained public slot; import cannot invent authority');
    if (entry.keyPresent) throw Error('Local key already present; reuse the existing identity without import');
    const ui = createInterface({ input: stdin, output: stdout });
    let confirmed = false;
    try {
      confirmed = await ui.question(`Restore ONLY the local key for ${agent}? Retained assignment/history remain unchanged; standby import never grants Start. [yes/no]: `) === 'yes';
    } finally { ui.close(); }
    if (!confirmed) return;
    const secret = await readAgentSecret();
    importSlotKey(dir, agent, secret);
    console.log(`Local key restored for ${agent}; retained assignment and history unchanged. Start still requires this host's assignment. No keys or sessions transferred.`);
  } else if (command === 'assignment-export') {
    const entries = installationSlots(resolve(text(args[0])));
    const entry = args[2] ? entries.find(e => e.agent === args[2]) : entries.length === 1 ? entries[0] : undefined;
    if (!entry) throw Error('Specify agent public key after export filename for multi-slot host');
    const state = object(readPrivate(entry.path));
    const genesis = validateGenesis(object(state.assignment).genesis);
    writePrivate(resolve(text(args[1])), genesis, true);
    console.log('Public genesis exported; no keys or execution grant. Provision the matching key separately on the destination.');
  } else if (command === 'migrate-assignment') {
    const dir = resolve(text(args[0]));
    const setup = validateSetup(readPrivate(join(dir, 'setup.json')));
    if (setup.agentSecret === undefined) throw Error('Legacy setup requires its agent key; explicit migrate-slots first');
    const ui = createInterface({ input: stdin, output: stdout });
    try {
      if ((await ui.question('Enroll this existing stopped journal as the UNIQUE authority? Verify no clones or unmanaged execution of this identity exist. Missing journals cannot be repaired here. [yes/no]: ')) !== 'yes') return;
      migrateAssignment(dir, createGenesis(publicKey(setup.ownerSecret), publicKey(setup.agentSecret), setup.host));
      console.log('Legacy journal pinned; lifecycle history retained. Do not clone or roll back this installation.');
    } finally { ui.close(); }
  } else if (command === 'conversation-setup') {
    const dir = resolve(text(args[0]));
    const lock = join(dir, 'host.lock');
    mkdirSync(lock, { mode: 0o700 }); // Same atomic exclusion as host startup; never remove a competing lock.
    try {
      const entries = installationSlots(dir);
      if (entries.some(e => object(readPrivate(e.path)).phase !== 'stopped')) throw Error('Reconcile every prior run before local setup changes');
      const setup = entries[0]!.setup;
      if (setup.mode !== 'buzz-agent-databricks-v2') throw Error('Provision Buzz Agent first; this action never creates or replaces agent keys');
      if (setup.agentSecret === undefined) throw Error('First slot key removed locally; conversation setup requires its agent key');
      const ui = createInterface({ input: stdin, output: stdout });
      try {
        const executable = realpathSync(text(await ui.question('Absolute installed buzz-acp executable: ')));
        const relay = text(await ui.question('Buzz CONVERSATION relay URL (not the Beehive management relay): '));
        const replyTool = (await ui.question('Enable the installed Buzz CLI tool under this agent identity? [yes/no]: ')) === 'yes' ? {
          executable: realpathSync(text(await ui.question('Absolute installed buzz CLI executable: '))),
        } : undefined;
        const conversation = { executable, relay, ...(replyTool ? { replyTool } : {}) };
        if (setup.agentSecret === undefined) throw Error('Slot key missing; cannot prepare conversation');
        const plan = prepareConversation(conversation, { executable: setup.runner, args: setup.args, workspace: setup.workspace, home: text(setup.serviceHome), configDirectory: text(setup.configDirectory), databricksHost: text(setup.databricksHost), model: 'databricks-claude-haiku-4-5' }, setup.agentSecret, publicKey(setup.ownerSecret));
        if ((await ui.question('Save onto existing identity? Start waits for an admitted conversation and local provider sign-in. [yes/no]: ')) !== 'yes') return;
        // One atomic replacement; keys, assignment and provider auth context unchanged.
        saveDefaultHarness(dir, { ...setup, conversation });
        console.log(`Conversation setup saved for existing agent ${plan.agentPublicKey}. No network connection, login or process started. The community operator must admit this public key and owner to the chosen relay/channels (or provision a valid owner attestation locally). Admission remains unverified. Start uses the host-owned ACP broker; the Buzz CLI tool, when enabled, uses agent membership authority across conversations without local thread configuration.`);
      } finally { ui.close(); }
    } finally { rmdirSync(lock); }
  } else if (command === 'auth-info') {
    const setup = installationSlots(resolve(text(args[0])))[0]!.setup;
    if (setup.mode !== 'buzz-agent-databricks-v2') throw Error('This harness setup has no provider sign-in');
    console.log(`Sign in on host ${setup.host} as the same OS user running its host service (current uid ${process.getuid?.() ?? 'unknown'}), not your Desktop account. Use exactly this context:\nHOME=${shellQuote(text(setup.serviceHome))} BUZZ_AGENT_CONFIG_DIR=${shellQuote(text(setup.configDirectory))} DATABRICKS_HOST=${shellQuote(text(setup.databricksHost))} ${shellQuote(setup.runner)} auth databricks\nBuzz Agent owns OAuth/cache/refresh. No login was initiated; Save and Stop do not require auth.`);
  } else if (command === 'relay') {
    const server = await relay(Number(args[0]),text(args[1]),resolve(text(args[2])));
    console.log(`Development relay listening ${JSON.stringify(server.address())}`);
    process.once('SIGINT',() => { for (const c of server.clients) c.close(); server.close(); });
  } else if (command === 'host') {
    const running = await host(resolve(text(args[0])),text(args[1]));
    console.log(`Host online; agents ${running.agents.join(', ')}. Ctrl-C stops owned runner before host exits.`);
    process.once('SIGINT',() => { void running.close(); });
    process.once('SIGTERM',() => { void running.close(); });
  } else if (command === 'tui') {
    const secret = text(object(readPrivate(resolve(text(args[0])))).secret);
    const inventory = new Map<string,Message>();
    const profiles = new Profiles();
    const client = managementClient(join(dirname(resolve(text(args[0]))), 'management-intents'),text(args[1]),secret,m => {
      profiles.receive(m);
      if (m.type === 'inventory') {
        const key = JSON.stringify([m.host, m.agent]);
        const prior = inventory.get(key);
        if (!prior || Number(m.body.observedAt) > Number(prior.body.observedAt)) inventory.set(key,m);
      }
    }, () => {
      console.log(client.connected ? '\nManagement relay connected.' : '\nRelay disconnected: pending results UNKNOWN; automatic reconnect is bounded (disabled on policy refusal). Use reconcile after checking relay policy.');
      for (const operation of client.status()) console.log(`${operationLabel(operation.request)}: ${operation.state} | ${operation.result ?? operation.publication}`);
    });
    try { await client.ready; } catch (error) { client.close(); throw error; }
    const ui = createInterface({ input: stdin, output: stdout });
    console.log('Beehive | Hosts → assigned agent → selected-next / actual run\nCommands: binding <local-id>, configurations, config-new, config-select <name>, config-rename, config-remove <name>, profiles, profile-new, profile-edit <number>, apply <number|default>, operations, reconcile, retry <number>, hosts, agents, select <number or unique host>, show, save, start, restart, stop, move, quit. Closing this UI does not stop hosts.');
    let selected = '';
    let profileRows: Profile[] = [];
    try {
      for (;;) {
        const line = (await ui.question('beehive> ')).trim();
        if (line === 'quit') break;
        if (line === 'operations') {
          client.status().forEach((o, index) => console.log(`${index + 1}. ${operationLabel(o.request)} at revision ${o.request.revision}: ${o.state} | ${o.result ?? o.publication}${o.retryAvailable ? ` | reconcile, then retry ${index + 1}` : ''}`));
          console.log('Historical results are not current host state. Reconcile queries receipts without retrying blocked work.'); continue;
        }
        if (line === 'reconcile') { client.reconcile(); console.log('Reconnecting/querying host results; blocked work stays blocked. Connection alone proves neither policy repair nor completion.'); continue; }
        if (line.startsWith('retry ')) {
          const number = line.slice(6);
          const operation = /^[1-9][0-9]*$/.test(number) ? client.status()[Number(number) - 1] : undefined;
          if (!operation?.retryAvailable) { console.log('Use operations to choose an unresolved policy-blocked operation.'); continue; }
          console.log(`Retry original ${operationLabel(operation.request)} at revision ${operation.request.revision} ONCE, with unchanged signature, ID and preconditions. It may already have executed. Reconcile first. Repair relay policy before retry; expired/invalid signed events cannot be repaired here. This does not cancel remote work or enable automatic retries.`);
          if ((await ui.question('Attempt unchanged operation after policy repair? [yes/no]: ')) === 'yes') {
            try { client.retry(operation.request.id); console.log('Unchanged retry attempted; result UNKNOWN until host receipt. Automatic retry stays disabled.'); }
            catch (error) { console.log(error instanceof Error ? error.message : 'Retry not sent'); }
          }
          continue;
        }
        if (line === 'hosts') { [...inventory.values()].forEach((m, index) => console.log(`${index + 1}. ${m.host} | agent ${m.agent} | assigned ${m.body.assignedHost} | ${m.body.phase} | ${m.body.readiness} | ${Date.now()-Number(m.body.observedAt) > 6000 ? 'STALE/UNKNOWN' : 'recent host report'}`)); continue; }
        if (line === 'agents') {
          const agents = new Set([...inventory.values()].map(m => m.agent));
          for (const agent of agents) console.log(`Agent ${agent}: ${[...inventory.values()].filter(m => m.agent === agent).map(m => `${m.host} (reports assigned ${m.body.assignedHost}, ${m.body.phase})`).join('; ')}`);
          console.log('Per-host reports are observations, not global liveness or consensus. Use hosts then select a numbered host/agent row.'); continue;
        }
        if (line.startsWith('select ')) {
          const choice = line.slice(7);
          const rows = [...inventory.entries()];
          const matches = /^[1-9][0-9]*$/.test(choice) ? rows.slice(Number(choice)-1, Number(choice)) : rows.filter(([,m]) => m.host === choice);
          if (matches.length !== 1) { console.log('Unknown or ambiguous host: use hosts and select its numbered host/agent row.'); continue; }
          selected = matches[0]![0]; console.log(`Selected ${matches[0]![1].host} agent ${matches[0]![1].agent}`); continue;
        }
        if (line === 'profiles') {
          profileRows = profiles.list();
          profileRows.forEach((p, i) => console.log(`${i + 1}. ${p.name} | revision ${p.revision.slice(0, 12)} | parent ${p.parent?.slice(0, 12) ?? 'new'} | ${p.instructions}`));
          for (const p of profiles.incomplete()) console.log(`INCOMPLETE ${p.name} @ ${p.revision.slice(0, 12)}: missing/conflicting lineage; cannot apply`);
          console.log('Immutable versions; concurrent edits remain branches. Publishing never applies to agents. No destructive deletion; apply default explicitly to clear.'); continue;
        }
        if (line === 'profile-new' || line.startsWith('profile-edit ')) {
          try {
            const parent = line === 'profile-new' ? undefined : profileRows[Number(line.slice(13)) - 1];
            if (line !== 'profile-new' && !parent) throw Error('Choose a profile version number from profiles');
            const name = parent?.name ?? await ui.question('Profile name: ');
            const instructions = await ui.question('Nonsecret behavior instructions (no provider/model/credentials): ');
            const value = profile({ name, parent: parent?.revision ?? null, instructions, revision: profileRevision(name, parent?.revision ?? null, instructions) });
            const affected = [...inventory.values()].filter(m => (m.body.selectedNext as any)?.behavior?.name === name);
            for (const m of affected) console.log(`Association unchanged: ${m.host} agent ${m.agent}`);
            console.log(`Publish ${name}: ${instructions}; ${affected.length} observed associations remain unchanged. Apply separately; no restart.`);
            if (await ui.question('Publish immutable revision? [yes/no]: ') === 'yes') client.submit(message('profile', 'profiles', 'profiles', 0, value));
          } catch (error) { console.log(error instanceof Error ? error.message : 'Publication failed'); }
          continue;
        }
        const current = inventory.get(selected);
        if (!current) { console.log('Select an advertised host first.'); continue; }
        if (line === 'show') {
          const next = current.body.selectedNext as any, actual = current.body.actualRun as any;
          const label = (s: any) => `${s?.configuration ? `${s.configuration.name} @ ${s.configuration.revision}` : 'default (legacy/unversioned)'} | behavior ${s?.behavior ? `${s.behavior.name} @ ${s.behavior.revision.slice(0, 12)}` : 'upstream default'}`;
          console.log(`Agent ${current.agent} | host ${current.host} | assigned ${current.body.assignedHost}\nCurrent: ${actual ? label(actual.selection) : 'no actual run'}\nSelected next: ${label(next)} (explicit Restart to apply)\n${JSON.stringify(current,null,2)}`); continue; }
        if (line === 'configurations') {
          console.log(`Host ${current.host} | agent ${current.agent} | named launch candidates (configuration is not execution permission)`);
          for (const [name, value] of Object.entries(object(current.body.configurations))) {
            const candidate = object(value), revision = object(candidate.configuration).revision;
            const active = name === ((current.body.selectedNext as any).configuration?.name ?? 'default');
            console.log(`${active ? '* selected-next' : '  saved'} ${name} @ ${revision} | ${candidate.model} | ${candidate.workspace} | behavior ${candidate.behavior ? object(candidate.behavior).name : 'upstream default'}`);
          }
          console.log('config-select <name> selects next; save edits it; explicit restart applies it.'); continue;
        }
        if (line === 'config-new' || line === 'config-rename' || line.startsWith('config-select ') || line.startsWith('config-remove ')) {
          if (Date.now() - Number(current.body.observedAt) > 6000) { console.log('Host stale; no action sent.'); continue; }
          try {
            let body: Record<string, unknown>;
            if (line === 'config-new') body = { configurationAction: 'create', name: await ui.question('New configuration name (copies selected-next, same identity/setup): ') };
            else if (line === 'config-rename') body = { configurationAction: 'rename', name: await ui.question('Existing configuration name: '), newName: await ui.question('New configuration name: ') };
            else { const [command, ...name] = line.split(' '); body = { configurationAction: command === 'config-select' ? 'select' : 'remove', name: name.join(' ') }; }
            if (await ui.question(`Update configuration on ${current.host} agent ${current.agent}, actual run unchanged? [yes/no]: `) !== 'yes') continue;
            client.submit(message('save', current.host, current.agent, current.revision, body));
            console.log('Named configuration pending host CAS acceptance. Save edits selected configuration; Start/Restart is separate.');
          } catch (error) { console.log(error instanceof Error ? error.message : 'Configuration not submitted'); }
          continue;
        }
        if (!line.startsWith('binding ') && !line.startsWith('apply ') && !['save','start','restart','stop','move'].includes(line)) { console.log('Use operations/reconcile/retry <number>/hosts/select/show/save/start/restart/stop/move/quit.'); continue; }
        if (Date.now()-Number(current.body.observedAt) > 6000) { console.log('Host stale: status unknown; no action sent.'); continue; }
        let body: Record<string, unknown> = {};
        let action = line;
        if (line.startsWith('binding ')) {
          const binding = (current.body.harnessSetups as any[]).find(b => b.id === line.slice(8));
          if (!binding) { console.log('Unknown binding; use show for advertised harnessSetups.'); continue; }
          const next = current.body.selectedNext as Record<string, unknown>;
          body = { ...next, harnessSetup: { id: binding.id, fingerprint: binding.fingerprint }, model: binding.models[0], workspace: binding.workspaces[0] };
          action = 'save';
        }
        if (line.startsWith('apply ')) {
          const choice = line.slice(6); const version = profileRows[Number(choice) - 1];
          if (choice !== 'default' && !version) { console.log('Choose a profile version number from profiles, or default.'); continue; }
          const next = current.body.selectedNext as Record<string, unknown>;
          body = { ...(next.harnessSetup ? { harnessSetup: next.harnessSetup } : {}), model: next.model, workspace: next.workspace, profile: version?.revision ?? 'default', ...(version ? { behavior: version } : {}) };
          action = 'save';
          console.log(`Apply selected-next to ${current.host} agent ${current.agent}: ${version?.name ?? 'default'} ${version?.revision.slice(0, 12) ?? ''}; actual run unchanged until Restart.`);
        }
        if (line === 'save') {
          console.log(`Allowed models: ${JSON.stringify(current.body.models)}; workspaces: ${JSON.stringify(current.body.workspaces)}; behavior profiles: ${JSON.stringify(current.body.profiles)}`);
          const model = await ui.question('Model: '), workspace = await ui.question('Workspace: '), choice = await ui.question('Behavior profile: ');
          const version = profileRows[Number(choice) - 1];
          if (choice !== 'default' && !version) { console.log('Use default or a version number from profiles.'); continue; }
          body = { ...((current.body.selectedNext as any).harnessSetup ? { harnessSetup: (current.body.selectedNext as any).harnessSetup } : {}), model, workspace, profile: version?.revision ?? 'default', ...(version ? { behavior: version } : {}) };
        }
        if (line === 'move') {
          const destinations = [...inventory.values()].filter(m => m.agent === current.agent && m.host !== current.host);
          destinations.forEach((m, i) => console.log(`${i + 1}. ${m.host} | ${JSON.stringify(m.body.selectedNext)} | locally provisioned key is NOT Start authority`));
          const target = destinations[Number(await ui.question('Destination number: ')) - 1];
          if (!target || Date.now()-Number(target.body.observedAt) > 6000) { console.log('Destination unknown/stale; repair local key/setup/auth and host connection. No Move sent.'); continue; }
          if (await ui.question(`Move agent ${current.agent} from ${current.host} to ${target.host}, fresh execution, no workspace/session/credentials transferred; source cannot resume after grant. Confirm [yes/no]: `) !== 'yes') continue;
          body = { target: target.host, targetRevision: target.revision, selection: { ...(target.body.selectedNext as object), profile: (current.body.selectedNext as any).profile, ...((current.body.selectedNext as any).behavior ? { behavior: (current.body.selectedNext as any).behavior } : { behavior: undefined }) } };
        }
        const request = message(action as 'save' | 'start' | 'restart' | 'stop' | 'move',current.host,current.agent,current.revision,body);
        try { client.submit(request); } catch (error) { console.log(error instanceof Error ? error.message : 'Operation not submitted'); continue; }
        console.log(`Durably pending ${line}: ${operationLabel(request)}; publication is NOT host acceptance.`);
      }
    } finally { ui.close(); client.close(); console.log('UI closed; host lifetime is independent.'); }
  } else console.log(help);
}
main().catch(error => { console.error(error instanceof Error ? error.message : 'Beehive failed'); process.exitCode = 1; });
