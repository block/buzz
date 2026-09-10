import { createInterface } from 'node:readline/promises';
import { stdin, stdout } from 'node:process';
import { existsSync, mkdirSync, realpathSync, rmdirSync } from 'node:fs';
import { resolve, join, dirname } from 'node:path';
import { relay } from './relay.ts';
import { host, validateSetup, provision, migrateAssignment } from './host.ts';
import { migrateSlots, addSlot, installationSlots, saveDefaultHarness } from './slots.ts';
import { managementClient } from './intents.ts';
import { message, newKey, publicKey, object, text, type Message } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { createGenesis, validateGenesis } from './assignment.ts';
import { prepareConversation } from './conversation.ts';

const shellQuote = (s: string) => `'${s.replaceAll("'", "'\"'\"'")}'`;
const [command, ...args] = process.argv.slice(2);
const help = `Beehive — isolated development preview (loopback relay only)
  identity <new-directory>                  Create a NEW local owner identity
  setup <new-host-directory> <identity-file> Guided local harness + key setup
  migrate-slots <host-directory>            Explicit stopped upgrade, preserves journal
  add-agent <host-directory>                New identity using shared local harness
  assignment-export <host-directory> <new-file> Export public pinned genesis locally
  migrate-assignment <host-directory>       Explicit stopped legacy enrollment
  auth-info <host-directory>                Print exact local sign-in context (no login)
  conversation-setup <host-directory>       Attach external buzz-acp to EXISTING identity (Start gated)
  relay <port> <owner-public-key> <log-file> Dedicated ciphertext relay
  host <host-directory> <ws://127.0.0.1:port> Persistent foreground host
  tui <identity-file> <ws://127.0.0.1:port>   Relay-connected terminal UI
setup/add-agent accept optional <local-key-file> <public-genesis-file> for standby import.
assignment-export accepts agent public key after filename when several slots exist.
No Move, provider login RPC or production relay support yet.`;
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
      const extra = mode === '1' ? text(await ui.question('Absolute fixture TypeScript script: ')) : '';
      const databricksHost = mode === '2' ? text(await ui.question('Databricks workspace HTTPS URL: ')) : undefined;
      if (databricksHost && new URL(databricksHost).protocol !== 'https:') throw Error('HTTPS workspace required');
      const importing = args.length > 2;
      const agentSecret = importing ? text(object(readPrivate(resolve(text(args[2])))).secret) : newKey();
      const genesis = importing ? validateGenesis(readPrivate(resolve(text(args[3])))) : createGenesis(publicKey(secret), publicKey(agentSecret), name);
      if (importing && genesis.initialHost === name) throw Error('Import cannot create a second initial authority installation');
      if ((await ui.question(importing ? `Provision matching key as standby only; assignment remains ${genesis.initialHost}? [yes/no]: ` : 'Create a NEW agent identity assigned exclusively to this host? [yes/no]: ')) !== 'yes') return;
      mkdirSync(dir,{ mode: 0o700 });
      if (mode === '2') { mkdirSync(join(dir,'service-home'),{ mode: 0o700 }); mkdirSync(join(dir,'agent-config'),{ mode: 0o700 }); }
      provision(dir,{ host: name, ownerSecret: secret, agentSecret, runner, args: mode === '1' ? [resolve(extra)] : [], workspace, mode: mode === '1' ? 'fixture' : 'buzz-agent-databricks-v2', ...(databricksHost ? { databricksHost, serviceHome: join(dir,'service-home'), configDirectory: join(dir,'agent-config') } : {}) }, genesis);
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
  } else if (command === 'assignment-export') {
    const entries = installationSlots(resolve(text(args[0])));
    const entry = args[2] ? entries.find(e => publicKey(e.setup.agentSecret) === args[2]) : entries.length === 1 ? entries[0] : undefined;
    if (!entry) throw Error('Specify agent public key after export filename for multi-slot host');
    const state = object(readPrivate(entry.path));
    const genesis = validateGenesis(object(state.assignment).genesis);
    writePrivate(resolve(text(args[1])), genesis, true);
    console.log('Public genesis exported; no keys or execution grant. Provision the matching key separately on the destination.');
  } else if (command === 'migrate-assignment') {
    const dir = resolve(text(args[0]));
    const setup = validateSetup(readPrivate(join(dir, 'setup.json')));
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
      const ui = createInterface({ input: stdin, output: stdout });
      try {
        const executable = realpathSync(text(await ui.question('Absolute installed buzz-acp executable: ')));
        const relay = text(await ui.question('Buzz CONVERSATION relay URL (not the Beehive management relay): '));
        const replyTool = (await ui.question('Enable the installed Buzz CLI tool under this agent identity? [yes/no]: ')) === 'yes' ? {
          executable: realpathSync(text(await ui.question('Absolute installed buzz CLI executable: '))),
        } : undefined;
        const conversation = { executable, relay, ...(replyTool ? { replyTool } : {}) };
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
    const client = managementClient(join(dirname(resolve(text(args[0]))), 'management-intents'),text(args[1]),secret,m => {
      if (m.type === 'inventory') {
        const key = JSON.stringify([m.host, m.agent]);
        const prior = inventory.get(key);
        if (!prior || Number(m.body.observedAt) > Number(prior.body.observedAt)) inventory.set(key,m);
      }
    }, () => {
      console.log(client.connected ? '\nManagement relay connected.' : '\nRelay disconnected: pending results UNKNOWN; automatic reconnect is bounded (disabled on policy refusal). Use reconcile after checking relay policy.');
      for (const operation of client.status()) console.log(`${operation.request.host} ${operation.request.type}: ${operation.state} | ${operation.result ?? operation.publication}`);
    });
    try { await client.ready; } catch (error) { client.close(); throw error; }
    const ui = createInterface({ input: stdin, output: stdout });
    console.log('Beehive | Hosts → assigned agent → selected-next / actual run\nCommands: operations, reconcile, retry <number>, hosts, agents, select <number or unique host>, show, save, start, stop, quit. Closing this UI does not stop hosts.');
    let selected = '';
    try {
      for (;;) {
        const line = (await ui.question('beehive> ')).trim();
        if (line === 'quit') break;
        if (line === 'operations') {
          client.status().forEach((o, index) => console.log(`${index + 1}. ${o.request.host} ${o.request.type} at revision ${o.request.revision}: ${o.state} | ${o.result ?? o.publication}${o.retryAvailable ? ` | reconcile, then retry ${index + 1}` : ''}`));
          console.log('Historical results are not current host state. Reconcile queries receipts without retrying blocked work.'); continue;
        }
        if (line === 'reconcile') { client.reconcile(); console.log('Reconnecting/querying host results; blocked work stays blocked. Connection alone proves neither policy repair nor completion.'); continue; }
        if (line.startsWith('retry ')) {
          const number = line.slice(6);
          const operation = /^[1-9][0-9]*$/.test(number) ? client.status()[Number(number) - 1] : undefined;
          if (!operation?.retryAvailable) { console.log('Use operations to choose an unresolved policy-blocked operation.'); continue; }
          console.log(`Retry original ${operation.request.host} ${operation.request.type} at revision ${operation.request.revision} ONCE, with unchanged signature, ID and preconditions. It may already have executed. Reconcile first. Repair relay policy before retry; expired/invalid signed events cannot be repaired here. This does not cancel remote work or enable automatic retries.`);
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
        const current = inventory.get(selected);
        if (!current) { console.log('Select an advertised host first.'); continue; }
        if (line === 'show') { console.log(JSON.stringify(current,null,2)); continue; }
        if (!['save','start','stop'].includes(line)) { console.log('Use operations/reconcile/retry <number>/hosts/select/show/save/start/stop/quit.'); continue; }
        if (Date.now()-Number(current.body.observedAt) > 6000) { console.log('Host stale: status unknown; no action sent.'); continue; }
        let body: Record<string, unknown> = {};
        if (line === 'save') {
          console.log(`Allowed models: ${JSON.stringify(current.body.models)}; workspaces: ${JSON.stringify(current.body.workspaces)}; behavior profiles: ${JSON.stringify(current.body.profiles)}`);
          body = { model: await ui.question('Model: '), workspace: await ui.question('Workspace: '), profile: await ui.question('Behavior profile: ') };
        }
        const request = message(line as 'save' | 'start' | 'stop',current.host,current.agent,current.revision,body);
        try { client.submit(request); } catch (error) { console.log(error instanceof Error ? error.message : 'Operation not submitted'); continue; }
        console.log(`Durably pending ${line} (${request.id}); publication is NOT host acceptance.`);
      }
    } finally { ui.close(); client.close(); console.log('UI closed; host lifetime is independent.'); }
  } else console.log(help);
}
main().catch(error => { console.error(error instanceof Error ? error.message : 'Beehive failed'); process.exitCode = 1; });
