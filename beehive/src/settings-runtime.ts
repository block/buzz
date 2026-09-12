import { readSettings, type Settings } from './settings.ts';
import { validateSetup, type Setup } from './host.ts';

/** Materialize reusable runtimes into an existing authorized slot's inventory.
 * Workspace, conversation authority and identity remain owned by that slot.
 * Catalog registration alone cannot create a slot or authorize execution. */
export function runtimeBindings(base: Setup, settings: Settings): Record<string, Setup> {
  if (!base.serviceHome || !base.configDirectory) return {};
  return Object.fromEntries(settings.runtimes.map(runtime => {
    const provider = settings.providers.find(p => p.id === runtime.providerId);
    if (!provider) throw Error('Runtime provider missing');
    return [`runtime:${runtime.id}`, validateSetup({
      host: base.host, ownerPublic: base.ownerPublic, ...(base.ownerSecret ? { ownerSecret: base.ownerSecret } : {}), ...(base.agentSecret ? { agentSecret: base.agentSecret } : {}),
      runner: runtime.executable, args: [], workspace: base.workspace, allowedWorkspaces: base.allowedWorkspaces,
      serviceHome: base.serviceHome, configDirectory: base.configDirectory,
      ...(base.conversation ? { conversation: base.conversation } : {}), mode: 'buzz-agent-api-key',
      buzzProvider: provider.type === 'databricks_v2' ? { provider:'databricks_v2',auth:'token',baseUrl:provider.endpoint,credential:provider.key,models:[runtime.model] } : { provider: 'openai-compat', baseUrl: provider.endpoint, credential: provider.key, wire: 'auto', models: [runtime.model] },
    })];
  }));
}
/** Complete validation precedes addition to an existing slot's binding map. */
export function loadRuntimeBindings(directory: string, base: Setup) { return runtimeBindings(base,readSettings(directory)); }
