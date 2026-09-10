import { createInterface } from 'node:readline/promises';
import { stdin, stdout } from 'node:process';
import { existsSync, mkdirSync, realpathSync, rmdirSync } from 'node:fs';
import { resolve, join, dirname } from 'node:path';
import { relay } from './relay.ts';
import { host, validateSetup } from './host.ts';
import { managementClient } from './intents.ts';
import { message, newKey, publicKey, object, text, type Message } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { prepareConversation } from './conversation.ts';

const shellQuote = (s: string) => `'${s.replaceAll("'", "'\"'\"'")}'`;
const [command, ...args] = process.argv.slice(2);
const help = `Beehive — isolated development preview (loopback relay only)
  identity <new-directory>                  Create a NEW local owner identity
  setup <new-host-directory> <identity-file> Guided local harness + key setup
  auth-info <host-directory>                Print exact local sign-in context (no login)
  conversation-setup <host-directory>       Attach external buzz-acp to EXISTING identity (Start gated)
  relay <port> <owner-public-key> <log-file> Dedicated ciphertext relay
  host <host-directory> <ws://127.0.0.1:port> Persistent foreground host
  tui <identity-file> <ws://127.0.0.1:port>   Relay-connected terminal UI
No import, Move, provider login RPC or production relay support yet.`;
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
      if ((await ui.question('Create a NEW agent identity assigned exclusively to this host? [yes/no]: ')) !== 'yes') return;
      mkdirSync(dir,{ mode: 0o700 });
      if (mode === '2') { mkdirSync(join(dir,'service-home'),{ mode: 0o700 }); mkdirSync(join(dir,'agent-config'),{ mode: 0o700 }); }
      writePrivate(join(dir,'setup.json'),{ host: name, ownerSecret: secret, agentSecret: newKey(), runner, args: mode === '1' ? [resolve(extra)] : [], workspace, mode: mode === '1' ? 'fixture' : 'buzz-agent-databricks-v2', ...(databricksHost ? { databricksHost, serviceHome: join(dir,'service-home'), configDirectory: join(dir,'agent-config') } : {}) });
      console.log('Local setup saved. Start the host separately; the TUI never owns its lifetime.');
      if (mode === '2') console.log(`Not authenticated. Run auth-info ${shellQuote(dir)} for the exact local host/service-user login command. Start uses an ACP greeting probe, not yet a Buzz relay conversation agent.`);
    } finally { ui.close(); }
  } else if (command === 'conversation-setup') {
    const dir = resolve(text(args[0]));
    const lock = join(dir, 'host.lock');
    mkdirSync(lock, { mode: 0o700 }); // Same atomic exclusion as host startup; never remove a competing lock.
    try {
      if (existsSync(join(dir, 'journal.json')) && object(readPrivate(join(dir, 'journal.json'))).phase !== 'stopped') throw Error('Reconcile the prior run before local setup changes');
      const setup = validateSetup(readPrivate(join(dir, 'setup.json')));
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
        writePrivate(join(dir, 'setup.json'), { ...setup, conversation });
        console.log(`Conversation setup saved for existing agent ${plan.agentPublicKey}. No network connection, login or process started. The community operator must admit this public key and owner to the chosen relay/channels (or provision a valid owner attestation locally). Admission remains unverified. Start uses the host-owned ACP broker; the Buzz CLI tool, when enabled, uses agent membership authority across conversations without local thread configuration.`);
      } finally { ui.close(); }
    } finally { rmdirSync(lock); }
  } else if (command === 'auth-info') {
    const setup = validateSetup(readPrivate(join(resolve(text(args[0])), 'setup.json')));
    if (setup.mode !== 'buzz-agent-databricks-v2') throw Error('This harness setup has no provider sign-in');
    console.log(`Sign in on host ${setup.host} as the same OS user running its host service (current uid ${process.getuid?.() ?? 'unknown'}), not your Desktop account. Use exactly this context:\nHOME=${shellQuote(text(setup.serviceHome))} BUZZ_AGENT_CONFIG_DIR=${shellQuote(text(setup.configDirectory))} DATABRICKS_HOST=${shellQuote(text(setup.databricksHost))} ${shellQuote(setup.runner)} auth databricks\nBuzz Agent owns OAuth/cache/refresh. No login was initiated; Save and Stop do not require auth.`);
  } else if (command === 'relay') {
    const server = await relay(Number(args[0]),text(args[1]),resolve(text(args[2])));
    console.log(`Development relay listening ${JSON.stringify(server.address())}`);
    process.once('SIGINT',() => { for (const c of server.clients) c.close(); server.close(); });
  } else if (command === 'host') {
    const running = await host(resolve(text(args[0])),text(args[1]));
    console.log(`Host online; agent ${running.agent}. Ctrl-C stops owned runner before host exits.`);
    process.once('SIGINT',() => { void running.close(); });
    process.once('SIGTERM',() => { void running.close(); });
  } else if (command === 'tui') {
    const secret = text(object(readPrivate(resolve(text(args[0])))).secret);
    const inventory = new Map<string,Message>();
    const client = managementClient(join(dirname(resolve(text(args[0]))), 'management-intents'),text(args[1]),secret,m => {
      if (m.type === 'inventory') {
        const prior = inventory.get(m.host);
        if (!prior || Number(m.body.observedAt) > Number(prior.body.observedAt)) inventory.set(m.host,m);
      }
    }, () => {
      console.log(client.connected ? '\nManagement relay connected.' : '\nRelay disconnected: pending results UNKNOWN; reconnect attempts are bounded.');
      for (const operation of client.status()) console.log(`${operation.request.host} ${operation.request.type}: ${operation.state} | ${operation.result ?? operation.publication}`);
    });
    try { await client.ready; } catch (error) { client.close(); throw error; }
    const ui = createInterface({ input: stdin, output: stdout });
    console.log('Beehive | Hosts → assigned agent → selected-next / actual run\nCommands: operations, hosts, select <host>, show, save, start, stop, quit. Closing this UI does not stop hosts.');
    let selected = '';
    try {
      for (;;) {
        const line = (await ui.question('beehive> ')).trim();
        if (line === 'quit') break;
        if (line === 'operations') { console.log(JSON.stringify(client.status(),null,2)); continue; }
        if (line === 'hosts') { for (const [name,m] of inventory) console.log(`${name}: ${m.body.phase} | ${m.body.readiness} | ${Date.now()-Number(m.body.observedAt) > 6000 ? 'STALE/UNKNOWN' : 'recent host report'}`); continue; }
        if (line.startsWith('select ')) { selected = line.slice(7); continue; }
        const current = inventory.get(selected);
        if (!current) { console.log('Select an advertised host first.'); continue; }
        if (line === 'show') { console.log(JSON.stringify(current,null,2)); continue; }
        if (!['save','start','stop'].includes(line)) { console.log('Use hosts/select/show/save/start/stop/quit.'); continue; }
        if (Date.now()-Number(current.body.observedAt) > 6000) { console.log('Host stale: status unknown; no action sent.'); continue; }
        let body: Record<string, unknown> = {};
        if (line === 'save') {
          console.log(`Allowed models: ${JSON.stringify(current.body.models)}; workspaces: ${JSON.stringify(current.body.workspaces)}; behavior profiles: ${JSON.stringify(current.body.profiles)}`);
          body = { model: await ui.question('Model: '), workspace: await ui.question('Workspace: '), profile: await ui.question('Behavior profile: ') };
        }
        const request = message(line as 'save' | 'start' | 'stop',selected,current.agent,current.revision,body);
        try { client.submit(request); } catch (error) { console.log(error instanceof Error ? error.message : 'Operation not submitted'); continue; }
        console.log(`Durably pending ${line} (${request.id}); publication is NOT host acceptance.`);
      }
    } finally { ui.close(); client.close(); console.log('UI closed; host lifetime is independent.'); }
  } else console.log(help);
}
main().catch(error => { console.error(error instanceof Error ? error.message : 'Beehive failed'); process.exitCode = 1; });
