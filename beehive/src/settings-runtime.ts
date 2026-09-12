import { codexHomeIdentity, codexHomePaths } from './codex-home.ts';
import { publicKey } from './protocol.ts';
import { readSettings, validateSettings, type Settings } from './settings.ts';
import { setupOwner, validateSetup, type Setup } from './host.ts';

/** Materialize reusable runtimes into an existing authorized slot's inventory.
 * Workspace, conversation authority and identity remain owned by that slot.
 * Catalog registration alone cannot create a slot or authorize execution. */
export function runtimeBindings(base: Setup, settings: Settings, agent = base.agentSecret ? publicKey(base.agentSecret) : undefined): Record<string, Setup> {
  validateSettings(settings);
  if (!base.serviceHome || !base.configDirectory) return {};
  return Object.fromEntries(settings.runtimes.map(runtime => {
    const provider = settings.providers.find(p => p.id === runtime.providerId);
    if (!provider) throw Error('Runtime provider missing');
    const managedHome = runtime.harness === 'codex' && base.serviceHome === base.configDirectory
      ? { parent: base.configDirectory!, identity: codexHomeIdentity(base.host, setupOwner(base), agent ?? '', runtime.id) } : undefined;
    const paths = managedHome ? codexHomePaths(managedHome) : undefined;
    return [`runtime:${runtime.id}`, validateSetup({
      host: base.host, ownerPublic: base.ownerPublic, ...(base.ownerSecret ? { ownerSecret: base.ownerSecret } : {}), ...(base.agentSecret ? { agentSecret: base.agentSecret } : {}),
      runner: runtime.executable, args: [], workspace: base.workspace, allowedWorkspaces: base.allowedWorkspaces,
      serviceHome: paths?.home ?? base.serviceHome, configDirectory: paths?.config ?? base.configDirectory,
      ...(base.conversation ? { conversation: base.conversation } : {}), ...(runtime.harness === 'codex' ? { mode: 'codex', codex: { ...(managedHome ? { managedHome } : {}), cli: runtime.cli!, credential: provider.key, models: [runtime.model] } } : { mode: runtime.harness === 'pi' ? 'pi' : 'buzz-agent-api-key', ...(runtime.harness === 'pi' ? { piCli: runtime.cli } : {}),
      buzzProvider: provider.type === 'databricks_v2' ? { provider:'databricks_v2',auth:'token',baseUrl:provider.endpoint,credential:provider.key,models:[runtime.model],...(runtime.effort ? { effort:runtime.effort } : {}) } : { provider: 'openai-compat', baseUrl: provider.endpoint, credential: provider.key, wire: 'auto', models: [runtime.model], ...(runtime.effort ? { effort:runtime.effort } : {}) } }),
    })];
  }));
}
/** Complete validation precedes addition to an existing slot's binding map. */
export function loadRuntimeBindings(directory: string, base: Setup, agent?: string) { return runtimeBindings(base,readSettings(directory),agent); }
