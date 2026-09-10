import { createInterface } from 'node:readline/promises';
import { stdin, stdout } from 'node:process';
import { existsSync, mkdirSync, realpathSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { relay } from './relay.ts';
import { host, validateSetup } from './host.ts';
import { connect } from './client.ts';
import { message, newKey, publicKey, object, text, type Message } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';

const shellQuote = (s: string) => `'${s.replaceAll("'", "'\"'\"'")}'`;
const [command, ...args] = process.argv.slice(2);
const help = `Beehive — isolated development preview (loopback relay only)
  identity <new-directory>                  Create a NEW local owner identity
  setup <new-host-directory> <identity-file> Guided local harness + key setup
  auth-info <host-directory>                Print exact local sign-in context (no login)
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
    const client = connect(text(args[1]),secret,m => {
      if (m.type === 'inventory') {
        const prior = inventory.get(m.host);
        if (!prior || Number(m.body.observedAt) > Number(prior.body.observedAt)) inventory.set(m.host,m);
      }
      if (m.type === 'receipt') console.log(`\nHost receipt (may be replayed) ${m.body.operation}: ${JSON.stringify(m.body)}`);
    });
    await client.ready;
    let intentionalClose = false;
    client.socket.on('close',() => { if (!intentionalClose) console.log('\nRelay disconnected: live status and unresolved requests UNKNOWN. Received receipts remain valid. Reopen TUI.'); });
    const ui = createInterface({ input: stdin, output: stdout });
    console.log('Beehive | Hosts → assigned agent → selected-next / actual run\nCommands: hosts, select <host>, show, save, start, stop, quit. Closing this UI does not stop hosts.');
    let selected = '';
    try {
      for (;;) {
        const line = (await ui.question('beehive> ')).trim();
        if (line === 'quit') break;
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
        client.send(request);
        console.log(`Requested ${line} (${request.id}); publication is NOT host acceptance.`);
      }
    } finally { intentionalClose = true; ui.close(); client.close(); console.log('UI closed; host lifetime is independent.'); }
  } else console.log(help);
}
main().catch(error => { console.error(error instanceof Error ? error.message : 'Beehive failed'); process.exitCode = 1; });
