import { provisionCredentialSlot, reconcileCredentialProvision } from './credential-slots.ts';
import { readHostIdentity } from './host-identity.ts';
import { verifyHostRegistration } from './host-registration.ts';
import { setupOwner } from './host.ts';
import { buzzProviderInput, buzzProviderGuidance, databricksOAuthGuidance } from './buzz-provider.ts';
import { customInput } from './custom-input.ts';
import { showPresets } from './presets.ts';
import { prepareAgent } from './acp.ts';
import { codexGuidance } from './codex.ts';
import { claudeGuidance } from './claude.ts';
import { readAgentSecret } from './key-input.ts';
import { conversationInput } from './conversation-input.ts';
import { localSetup } from './local-setup.ts';
import { enrollmentInput } from './enrollment-input.ts';
import { verifyHostCatalog, type HostCatalog } from './host-catalog.ts';
import { Profiles, profile, profileRevision, type Profile } from './profiles.ts';
import { createInterface } from 'node:readline/promises';
import { stdin, stdout } from 'node:process';
import { existsSync, mkdirSync, realpathSync, rmdirSync } from 'node:fs';
import { resolve, join, dirname } from 'node:path';
import { relay } from './relay.ts';
import { host, validateSetup, provision, migrateAssignment, setupModels } from './host.ts';
import { migrateSlots, addSlot, installationSlots, removeSlotKey, importSlotKey } from './slots.ts';
import { managementClient } from './intents.ts';
import { message, newKey, publicKey, object, text, type Message } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { createGenesis, validateGenesis } from './assignment.ts';
import { prepareConversation } from './conversation.ts';

const operationLabel = (m: Message) => `${m.host} ${m.type}${m.type === 'move' ? ` → ${String(m.body.target)}` : ''} | agent ${m.agent} | operation ${m.id}`;
const shellQuote = (s: string) => `'${s.replaceAll("'", "'\"'\"'")}'`;
const [command, ...args] = process.argv.slice(2);
const help = `Beehive — isolated development preview (loopback relay only)
  identity                                 Disabled: use your existing owner signer
  setup <host-directory>                     Offline host pairing/owner approval/import
  setup <host-directory> <identity-file>     LEGACY loopback diagnostic setup (owner key copied)
  provision-agent <host-directory> <binding-file> <genesis-file> Hidden matching agent import; OS credentials
  reconcile-provision <host-directory> <binding-file> <genesis-file> Explicit exact first-provision recovery
  catalog <new-file> <registration-files...> Retain verified public host registrations
  presets                                  Local process-free preset discovery/setup guidance
  local-setup <host-directory>              Bindings; new/reuse/hidden standby/restore identity
  migrate-slots <host-directory>            Explicit stopped upgrade, preserves journal
  add-agent <host-directory>                New identity using shared local harness
  remove-agent-key <host-directory> [agent-public-key] Remove ONE local key copy (public slot retained)
  import-agent-key <host-directory> <agent-public-key> Restore retained identity via hidden local entry
  assignment-export <host-directory> <new-file> Export public pinned genesis locally
  migrate-assignment <host-directory>       Explicit stopped legacy enrollment
  auth-info <host-directory> [binding-id]   Print local harness service context (no login)
  conversation-setup <host-directory>       Legacy guidance; use local-setup action normal
  relay <port> <owner-public-key> <log-file> Dedicated ciphertext relay
  host <host-directory> <ws://127.0.0.1:port> Persistent foreground host
  tui <identity-file> <ws://127.0.0.1:port>   Relay-connected terminal UI
setup/add-agent accept optional <local-key-file> <public-genesis-file> for standby import.
assignment-export accepts agent public key after filename when several slots exist.
Move is fixture-only experimental; containment acceptance remains gated. No provider login RPC or production relay support.`;
async function main() {
  if (command === 'identity') {
    throw Error('Standalone Beehive uses your existing owner identity through explicit secure input. Creating a parallel controller identity or persisting a plaintext owner key is disabled.');
  } else if (command === 'provision-agent' || command === 'reconcile-provision') {
    const directory = resolve(text(args[0]));
    const identity = readHostIdentity(directory);
    verifyHostRegistration(identity.registration, identity.pairing);
    const binding = object(readPrivate(resolve(text(args[1]))));
    if (['host', 'ownerSecret', 'ownerPublic', 'agentSecret'].some(key => Object.hasOwn(binding, key))) throw Error('Local binding file must not carry identity');
    const setup = validateSetup({ ...binding, host: identity.pairing.host, ownerPublic: identity.pairing.owner });
    const genesis = validateGenesis(readPrivate(resolve(text(args[2]))));
    const secret = await readAgentSecret();
    if (command === 'reconcile-provision') reconcileCredentialProvision(directory, setup, secret, genesis);
    else provisionCredentialSlot(directory, setup, secret, genesis);
    console.log(`Provisioned STOPPED public slot ${publicKey(secret)}; owner ${identity.pairing.owner}. Key stored and read-back verified in Beehive OS credential namespace. No Start or relay admission performed.`);
  } else if (command === 'presets') {
    showPresets();
  } else if (command === 'setup') {
    const dir = resolve(text(args[0]));
    if (args.length === 1 && !existsSync(join(dir, 'setup.json'))) {
      await enrollmentInput(dir); return;
    }
    if (existsSync(dir)) {
      if (args.length !== 1) throw Error('Existing setup: supply host directory only; retained owner cannot be replaced');
      await localSetup(dir); return;
    }
    const secret = text(object(readPrivate(resolve(text(args[1])))).secret);
    let ui = createInterface({ input: stdin, output: stdout });
    try {
      const name = text(await ui.question('Host name: '));
      let mode = await ui.question('Setup [1 deterministic fixture / 2 Buzz Agent + Databricks v2 / 3 Goose / 4 Claude Code / 5 Codex / 6 custom ACP / 7 preset discovery / 8 Buzz Agent provider (API key / Databricks token or OAuth)]: ');
      if (mode === '7') { showPresets(); console.log('Use existing local-setup add-preset for immutable diagnostic registration. No conversation identity created.'); return; }
      const custom = mode === '6' ? await customInput(ui) : undefined;
      if (custom) {
        if (custom.contract !== 'goose-native') throw Error('Diagnostic custom has no conversation model/profile contract; use existing local-setup add-custom. No identity created.');
        mode = '3'; // Explicit source-supported contract owner, not arbitrary ACP compatibility.
      }
      if (!['1','2','3','4','5','8'].includes(mode)) throw Error('Choose 1, 2, 3, 4 or 5');
      if (mode === '5') console.log(codexGuidance);
      if (mode === '4') console.log(claudeGuidance);
      const runner = custom ? realpathSync(custom.executable) : realpathSync(text(await ui.question(mode === '1' ? 'Absolute fixture runner executable: ' : mode === '5' ? 'Absolute installed codex-acp adapter: ' : mode === '4' ? 'Absolute installed claude-agent-acp adapter: ' : mode === '3' ? 'Absolute installed Goose executable (runs acp): ' : 'Absolute installed buzz-agent executable: ')));
      const buzzProvider = mode === '8' ? await buzzProviderInput(ui) : undefined;
      const claude = mode === '4' ? { cli: realpathSync(text(await ui.question('Absolute installed claude CLI: '))), apiKeyFile: text(await ui.question('Absolute owner-only local ANTHROPIC_API_KEY file (contents never relayed): ')), models: text(await ui.question('Operator-approved compatible exact Claude model IDs (comma-separated): ')).split(',').map(m => m.trim()) } : undefined;
      const codex = mode === '5' ? { cli: realpathSync(text(await ui.question('Absolute installed codex CLI: '))), apiKeyFile: text(await ui.question('Absolute owner-only local OPENAI_API_KEY file (contents never relayed): ')), models: text(await ui.question('Operator-approved compatible exact Codex model IDs (comma-separated): ')).split(',').map(m => m.trim()) } : undefined;
      const workspace = realpathSync(text(await ui.question('Allowed workspace (absolute directory): ')));
      const additionalWorkspace = await ui.question('Additional allowed workspace (blank for none): ');
      const allowedWorkspaces = additionalWorkspace ? [workspace, realpathSync(text(additionalWorkspace))] : [workspace];
      const extra = mode === '1' ? text(await ui.question('Absolute fixture TypeScript script: ')) : '';
      const databricksHost = mode === '2' ? text(await ui.question('Databricks workspace HTTPS URL: ')) : undefined;
      if (databricksHost) { const url = new URL(databricksHost); if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash || url.pathname !== '/') throw Error('Databricks workspace must be an HTTPS origin without credentials'); }
      console.log(mode === '8' ? buzzProviderGuidance : mode === '1' ? 'Fixture setup: no provider/login; trusted local executable, no agent credentials passed.' : mode === '5' ? codexGuidance : mode === '4' ? claudeGuidance : mode === '3' ? 'Goose setup: own provider configuration, not Buzz Agent OAuth.' : `Buzz Agent Databricks v2 owns browser OAuth and refresh. Intended host/service credential context: uid ${process.getuid?.() ?? 'unknown'}, HOME=${shellQuote(join(dir, 'service-home'))}, BUZZ_AGENT_CONFIG_DIR=${shellQuote(join(dir, 'agent-config'))}, DATABRICKS_HOST=${shellQuote(databricksHost!)}. Run ${shellQuote(runner)} auth databricks in exactly that context as the service user; no login initiated. Missing executable: install buzz-agent locally. Missing auth: use auth-info after setup.`);
      const gooseProvider = mode === '3' ? text(await ui.question('Locally configured Goose provider ID: ')) : undefined;
      const gooseModels = mode === '3' ? text(await ui.question('Operator-approved compatible exact model IDs (comma-separated): ')).split(',').map(m => m.trim()) : undefined;
      if (mode === '3') console.log(`Goose owns provider credentials and ~/.config/goose/config.yaml under dedicated HOME=${join(dir, 'service-home')}. Configure locally as the host service OS user; not Desktop HOME or Buzz Agent OAuth. No login, authentication or catalog verified. GOOSE_MODE=auto; exact model fixed on fresh launch.`);
      if (buzzProvider?.auth === 'external-oauth') console.log(databricksOAuthGuidance(runner, join(dir, 'service-home'), join(dir, 'agent-config'), buzzProvider.baseUrl));
      const purpose = mode === '1' ? 'diagnostic' : (await ui.question('Purpose [normal (default) / diagnostic ACP probe / cancel]: ')) || 'normal';
      if (purpose === 'cancel') return;
      if (!['normal', 'diagnostic'].includes(purpose)) throw Error('Choose normal, diagnostic or cancel');
      const conversation = purpose === 'normal' ? await conversationInput(ui) : undefined;
      const importing = args.length > 2;
      let agentSecret = importing ? text(object(readPrivate(resolve(text(args[2])))).secret) : newKey();
      let genesis = importing ? validateGenesis(readPrivate(resolve(text(args[3])))) : createGenesis(publicKey(secret), publicKey(agentSecret), name);
      if (importing && genesis.initialHost === name) throw Error('Import cannot create a second initial authority installation');
      const identityAction = await ui.question(importing ? `Provision matching key as standby only; assignment remains ${genesis.initialHost}? [yes/no]: ` : 'Create a NEW agent identity assigned exclusively to this host? [yes/no]: (or enter import-standby)  ');
      if (!importing && identityAction === 'import-standby') {
        genesis = validateGenesis(readPrivate(realpathSync(text(await ui.question('Local public genesis file (no key): ')))));
        if (genesis.owner !== publicKey(secret) || genesis.initialHost === name) throw Error('Standby requires matching owner and another initial host');
        if (await ui.question(`Import exact ${genesis.agent}; Start remains assigned to ${genesis.initialHost}? [yes/no]: `) !== 'yes') return;
        ui.close(); agentSecret = await readAgentSecret();
        if (publicKey(agentSecret) !== genesis.agent) throw Error('Imported key does not match public genesis');
        ui = createInterface({ input: stdin, output: stdout });
      } else if (identityAction !== 'yes') return;
      mkdirSync(dir,{ mode: 0o700 });
      if (mode !== '1') { mkdirSync(join(dir,'service-home'),{ mode: 0o700 }); mkdirSync(join(dir,'agent-config'),{ mode: 0o700 }); }
      const setup = validateSetup({ host: name, ownerSecret: secret, agentSecret, runner, ...(custom ? { custom } : {}), args: custom ? custom.args : mode === '1' ? [resolve(extra)] : mode === '3' ? ['acp'] : [], workspace, allowedWorkspaces, mode: mode === '8' ? 'buzz-agent-api-key' : mode === '1' ? 'fixture' : mode === '5' ? 'codex' : mode === '4' ? 'claude' : mode === '3' ? 'goose' : 'buzz-agent-databricks-v2', ...(buzzProvider ? { buzzProvider, serviceHome: join(dir,'service-home'), configDirectory: join(dir,'agent-config') } : {}), ...(codex ? { codex, serviceHome: join(dir,'service-home'), configDirectory: join(dir,'agent-config') } : {}), ...(claude ? { claude, serviceHome: join(dir,'service-home'), configDirectory: join(dir,'service-home') } : {}), ...(mode === '3' ? { gooseProvider, gooseModels, serviceHome: join(dir,'service-home'), configDirectory: join(dir,'service-home') } : {}), ...(databricksHost ? { databricksHost, serviceHome: join(dir,'service-home'), configDirectory: join(dir,'agent-config') } : {}), ...(conversation ? { conversation } : {}) });
      const lock = join(dir, 'host.lock'); mkdirSync(lock, { mode: 0o700 });
      try {
        if (setup.mode === 'codex') prepareAgent({ executable: runner, args: [], workspace, home: text(setup.serviceHome), configDirectory: text(setup.configDirectory), databricksHost: '', harness: 'codex', codex: setup.codex, model: setupModels(setup)[0]! });
        if (conversation) prepareConversation(conversation, { executable: runner, args: setup.args, workspace, home: text(setup.serviceHome), configDirectory: text(setup.configDirectory), ...(setup.buzzProvider ? { buzzProvider: setup.buzzProvider } : {}), databricksHost: setup.mode === 'buzz-agent-api-key' || setup.mode === 'goose' || setup.mode === 'claude' || setup.mode === 'codex' ? '' : text(setup.databricksHost), ...(setup.mode === 'codex' ? { harness: 'codex' as const, codex: setup.codex } : {}), ...(setup.mode === 'claude' ? { harness: 'claude' as const, claude: setup.claude } : {}), ...(setup.mode === 'goose' ? { harness: 'goose' as const, provider: setup.gooseProvider, ...(setup.custom ? { custom: setup.custom } : {}) } : {}), model: setupModels(setup)[0]! }, agentSecret, publicKey(secret));
        provision(dir, setup, genesis);
      } finally { rmdirSync(lock); }
      migrateSlots(dir);
      console.log(`Harness setup default is reusable; agent ${publicKey(agentSecret)} is independent. Host must remain stopped for local structural changes.`);
      while ((await ui.question('Add another independent NEW agent using this same harness setup? [yes/no]: ')) === 'yes') {
        const additional = newKey();
        addSlot(dir, additional, createGenesis(publicKey(secret), publicKey(additional), name));
        console.log(`Added agent ${publicKey(additional)} using harness setup default; no provider setup or key copy.`);
      }
      console.log('Local setup saved. Start the host separately; the TUI never owns its lifetime.');
      if (conversation) console.log('Normal conversation configured with installed runtime and Buzz CLI tool. No separate conversion command needed. Start remains gated on local provider setup, exact model evidence and relay admission; standby key possession still grants no Start.');
      if (mode !== '1' && !conversation) console.log(`Not authenticated. Run auth-info ${shellQuote(dir)} for the exact local host/service-user context. Start uses an ACP greeting probe, not yet a Buzz relay conversation agent. Conversation transport still requires explicit local-setup ${shellQuote(dir)}, action normal, with installed buzz-acp, a trusted conversation relay and optional installed Buzz CLI tool; no provider login or community admission has been verified.`);
    } finally { ui.close(); }
  } else if (command === 'local-setup') {
    if (args.length !== 1) throw Error('Use host directory only; no private key arguments');
    await localSetup(resolve(text(args[0])));
  } else if (command === 'migrate-slots') {
    const dir = resolve(text(args[0]));
    const ui = createInterface({ input: stdin, output: stdout });
    try {
      if (await ui.question('Upgrade this stopped installation to shared agent slots, preserving keys and journal? [yes/no]: ') !== 'yes') return;
      migrateSlots(dir); console.log('Slots enabled; original assignment and lifecycle history retained.');
    } finally { ui.close(); }
  } else if (command === 'add-agent') {
    const dir = resolve(text(args[0]));
    if (object(readPrivate(join(dir, 'setup.json'))).version === 3) {
      if (args.length > 2) throw Error('V3 add-agent accepts only an optional public genesis file; never a private key file');
      // Public-only overview: adding an independent identity does not read sibling keys.
      const first = installationSlots(dir, { read: () => null, create() { throw Error('Read only'); }, remove() { throw Error('Read only'); } })[0]!.setup;
      const importedRoot = args[1] ? validateGenesis(readPrivate(resolve(text(args[1])))) : undefined;
      if (importedRoot && (importedRoot.owner !== setupOwner(first) || importedRoot.initialHost === first.host)) throw Error('Standby requires matching owner and another initial host');
      const ui = createInterface({ input: stdin, output: stdout });
      let confirmed: boolean;
      try { confirmed = await ui.question(importedRoot ? `Import exact standby ${importedRoot.agent}, still assigned to ${importedRoot.initialHost}? [yes/no]: ` : 'Create an independent NEW v3 agent using OS credentials and default binding? [yes/no]: ') === 'yes'; }
      finally { ui.close(); }
      if (!confirmed) return;
      const secret = importedRoot ? await readAgentSecret() : newKey();
      const root = importedRoot ?? createGenesis(setupOwner(first), publicKey(secret), first.host);
      addSlot(dir, secret, root);
      console.log(`Added STOPPED public slot ${publicKey(secret)}; credential read-back verified; no relay admission or Start.`);
      return;
    }
    const first = installationSlots(dir)[0]!.setup;
    const importing = args.length > 1;
    const secret = importing ? text(object(readPrivate(resolve(text(args[1])))).secret) : newKey();
    const root = importing ? validateGenesis(readPrivate(resolve(text(args[2])))) : createGenesis(setupOwner(first), publicKey(secret), first.host);
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
      migrateAssignment(dir, createGenesis(setupOwner(setup), publicKey(setup.agentSecret), setup.host));
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
      if (setup.mode === 'fixture') throw Error('Provision an ACP harness first; this action never creates or replaces agent keys');
      if (setup.agentSecret === undefined) throw Error('First slot key removed locally; conversation setup requires its agent key');
      throw Error('Immutable bindings cannot be rewritten: use migrate-slots first for a legacy installation, then local-setup action normal to convert the selected ACP binding to a NEW reference');
    } finally { rmdirSync(lock); }
  } else if (command === 'auth-info') {
    const entries = installationSlots(resolve(text(args[0])));
    const setup = args[1] ? entries[0]!.bindings[text(args[1])] : entries[0]!.setup;
    if (!setup) throw Error('Unknown local binding');
    if (setup.mode === 'diagnostic-acp') { console.log('Setup/diagnostic only: authentication unverified, no model/native profile contract. Configure locally as the service user; no login/probe was run.'); return; }
    if (setup.mode === 'codex') {
      console.log(codexGuidance);
      console.log(`Dedicated HOME=${setup.serviceHome}; CODEX_HOME=${setup.configDirectory}. Save/Stop need no provider login.`); return;
    }
    if (setup.mode === 'claude') {
      console.log(claudeGuidance);
      console.log(`Service HOME=${shellQuote(text(setup.serviceHome))}; same host OS user required. Auth remains unverified; no credentials printed.`);
      return;
    }
    if (setup.mode === 'goose') {
      console.log(`Goose on host ${setup.host}: run/configure the installed Goose CLI ${shellQuote(setup.runner)} as the host service OS user (current uid ${process.getuid?.() ?? 'unknown'}) with HOME=${shellQuote(text(setup.serviceHome))}. Goose owns ~/.config/goose/config.yaml and provider credentials; GOOSE_PROVIDER=${shellQuote(text(setup.gooseProvider))}, GOOSE_MODE=auto; exact GOOSE_MODEL is selected remotely from approved models. Do not reuse Desktop HOME or Buzz Agent OAuth caches. No login/status command or authentication was inferred; executable found is not authenticated. Save/Stop need no provider login.`);
      return;
    }
    if (setup.mode === 'buzz-agent-api-key') { console.log(setup.buzzProvider?.auth === 'external-oauth' ? databricksOAuthGuidance(setup.runner, text(setup.serviceHome), text(setup.configDirectory), setup.buzzProvider.baseUrl) : buzzProviderGuidance); return; }
    if (setup.mode !== 'buzz-agent-databricks-v2') throw Error('This harness setup has no provider sign-in');
    console.log(`Sign in on host ${setup.host} as the same OS user running its host service (current uid ${process.getuid?.() ?? 'unknown'}), not your Desktop account. Use exactly this context:\nHOME=${shellQuote(text(setup.serviceHome))} BUZZ_AGENT_CONFIG_DIR=${shellQuote(text(setup.configDirectory))} DATABRICKS_HOST=${shellQuote(text(setup.databricksHost))} ${shellQuote(setup.runner)} auth databricks\nBuzz Agent owns OAuth/cache/refresh. No login was initiated; Save and Stop do not require auth.`);
  } else if (command === 'relay') {
    const server = await relay(Number(args[0]),text(args[1]),resolve(text(args[2])));
    console.log(`Development relay listening ${JSON.stringify(server.address())}`);
    let closing = false;
    const close = () => {
      if (closing) return; closing = true;
      for (const c of server.clients) c.close();
      server.close(error => {
        if (error) { console.error('Relay shutdown failed; storage durability may be uncertain. Retain history for reconciliation.'); process.exitCode = 1; }
      });
    };
    process.on('SIGINT', close); process.on('SIGTERM', close);
  } else if (command === 'host') {
    const controller = new AbortController();
    const stop = () => controller.abort();
    process.on('SIGINT', stop); process.on('SIGTERM', stop);
    try {
      const running = await host(resolve(text(args[0])), text(args[1]), controller.signal);
      console.log(`Host online; agents ${running.agents.join(', ')}. Ctrl-C stops owned runner before host exits.`);
      // The host owns teardown both before and after ready. Observe its one
      // completion promise so uncertain teardown cannot disappear as a rejection.
      await new Promise<void>(resolve => {
        if (controller.signal.aborted) resolve();
        else controller.signal.addEventListener('abort', () => resolve(), { once: true });
      });
      await running.close();
    } catch (error) {
      if (!controller.signal.aborted || error !== controller.signal.reason) throw error;
      // Transport ready rejection caused by intentional startup cancellation.
    } finally { process.off('SIGINT', stop); process.off('SIGTERM', stop); }
  } else if (command === 'catalog') {
    if (args.length < 2) throw Error('Catalog requires private registration files');
    const registrations = args.slice(1).map(file => readPrivate(resolve(text(file))));
    const first = object(object(registrations[0]).request);
    const catalog = verifyHostCatalog({ version: 1, owner: first.owner, relay: first.relay, registrations }, text(first.owner), text(first.relay));
    writePrivate(resolve(text(args[0])), catalog, true);
    console.log(`Retained ${catalog.registrations.length} verified host registrations. Public keys only; admission pending. Labels are not authority.`);
  } else if (command === 'tui') {
    const identity = object(readPrivate(resolve(text(args[0]))));
    const catalog: HostCatalog | undefined = 'registrations' in identity ? verifyHostCatalog(identity, text(identity.owner), text(args[1])) : undefined;
    const secret = catalog ? await readAgentSecret('Owner') : text(identity.secret);
    if (catalog && publicKey(secret) !== catalog.owner) throw Error('Wrong catalog owner signer');
    const inventory = new Map<string,Message>();
    const profiles = new Profiles();
    const client = managementClient(join(dirname(resolve(text(args[0]))), 'management-intents'),text(args[1]),secret,m => {
      profiles.receive(m);
      if (m.type === 'inventory') {
        const key = JSON.stringify([m.host, m.agent]);
        const prior = inventory.get(key);
        if (!prior && inventory.size >= 1000) return;
        if (!prior || Number(m.body.observedAt) > Number(prior.body.observedAt)) inventory.set(key,m);
      }
    }, () => {
      console.log(client.connected ? '\nManagement relay connected.' : '\nRelay disconnected: pending results UNKNOWN; automatic reconnect is bounded (disabled on policy refusal). Use reconcile after checking relay policy.');
      for (const operation of client.status()) console.log(`${operationLabel(operation.request)}: ${operation.state} | ${operation.result ?? operation.publication}`);
    }, catalog ? { catalog } : undefined);
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
        if (line === 'hosts') {
          if (catalog) for (const r of catalog.registrations) {
            const expired = Date.now() >= r.expires * 1000;
            console.log(`${r.request.label} | host ${r.request.host} | ${expired ? 'REGISTRATION EXPIRED / UNKNOWN' : [...inventory.values()].some(m => m.host === r.request.host) ? 'registered infrastructure' : 'UNREACHABLE / UNKNOWN (no host report)'}`);
          }
          [...inventory.values()].forEach((m, index) => console.log(`${index + 1}. ${m.host} | agent ${m.agent} | assigned ${m.body.assignedHost} | ${m.body.phase} | ${m.body.readiness} | ${Date.now()-Number(m.body.observedAt) > 6000 || (catalog && !catalog.registrations.some(r => r.request.host === m.host && Date.now() < r.expires * 1000)) ? 'STALE/UNKNOWN' : 'recent host report'}`)); continue;
        }
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
          if (catalog && (Date.now() - Number(current.body.observedAt) > 6000 || !catalog.registrations.some(r => r.request.host === current.host && Date.now() < r.expires * 1000))) console.log('STALE/UNKNOWN: the following is a historical host report, not current actual state.');
          const next = current.body.selectedNext as any, actual = current.body.actualRun as any;
          const label = (s: any) => `${s?.configuration ? `${s.configuration.name} @ ${s.configuration.revision}` : 'default (legacy/unversioned)'} | behavior ${s?.behavior ? `${s.behavior.name} @ ${s.behavior.revision.slice(0, 12)}` : 'upstream default'}`;
          console.log(`Agent ${current.agent} | host ${current.host} | assigned ${current.body.assignedHost}\nCurrent: ${actual ? label(actual.selection) : 'no actual run'}\nSelected next: ${label(next)} (explicit Restart to apply)\n${JSON.stringify(current,null,2)}`); continue; }
        if (line === 'configurations') {
          console.log(`Host ${current.host} | agent ${current.agent} | named launch candidates (configuration is not execution permission)`);
          for (const [name, value] of Object.entries(object(current.body.configurations))) {
            const candidate = object(value), revision = object(candidate.configuration).revision;
            const active = name === ((current.body.selectedNext as any).configuration?.name ?? 'default');
            const binding = (current.body.harnessSetups as any[]).find(b => b.id === (candidate.harnessSetup ? object(candidate.harnessSetup).id : current.body.defaultHarnessSetup ?? 'default'));
            console.log(`${binding?.availability === 'retired' ? '[retired binding; unavailable] ' : ''}${active ? '* selected-next' : '  saved'} ${name} @ ${revision} | ${candidate.model} | ${candidate.workspace} | behavior ${candidate.behavior ? object(candidate.behavior).name : 'upstream default'}`);
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
          if (binding?.availability === 'retired') { console.log('Binding retired; choose an available binding.'); continue; }
          if (binding?.availability === 'diagnostic-only') { console.log(binding.reason); continue; }
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
