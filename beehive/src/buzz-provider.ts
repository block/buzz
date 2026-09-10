import { readFileSync, realpathSync, statSync } from 'node:fs';
import { isAbsolute } from 'node:path';

/** Local immutable Buzz Agent provider contracts, pinned config.rs:631–691,922–960. */
export type BuzzProvider = { provider: 'anthropic' | 'openai-compat' | 'openrouter' | 'databricks_v2'; apiKeyFile?: string; auth?: 'token' | 'external-oauth'; baseUrl: string; models: string[]; wire?: 'auto' | 'chat' | 'responses' };
const contracts = {
  databricks_v2: { key: 'DATABRICKS_TOKEN', url: 'DATABRICKS_HOST' },
  anthropic: { key: 'ANTHROPIC_API_KEY', url: 'ANTHROPIC_BASE_URL' },
  'openai-compat': { key: 'OPENAI_COMPAT_API_KEY', url: 'OPENAI_COMPAT_BASE_URL' },
  openrouter: { key: 'OPENROUTER_API_KEY', url: 'OPENROUTER_BASE_URL' },
} as const;
/** Validate without reading credentials, contacting endpoints or initiating login. */
export function validateBuzzProvider(value: BuzzProvider): void {
  if (!value || !Object.hasOwn(contracts, value.provider) || Object.keys(value).some(k => !['provider', 'apiKeyFile', 'baseUrl', 'models', 'wire', 'auth'].includes(k)) || !Array.isArray(value.models) || !value.models.length || value.models.length > 100 || value.models.some(m => typeof m !== 'string' || !/^[a-zA-Z0-9_.:/-]{1,200}$/.test(m))) throw Error('Invalid local Buzz Agent provider/key-file/models binding');
  if (value.provider === 'databricks_v2' ? !['token', 'external-oauth'].includes(value.auth ?? '') : value.auth !== undefined) throw Error('Unsupported provider authentication contract');
  if (value.auth === 'external-oauth' ? value.apiKeyFile !== undefined : typeof value.apiKeyFile !== 'string' || !isAbsolute(value.apiKeyFile)) throw Error('Provider requires owner-only key file; external OAuth forbids token fallback');
  let url: URL;
  try { url = new URL(value.baseUrl); } catch { throw Error('Invalid local provider endpoint'); }
  if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash) throw Error('Provider endpoint requires HTTPS without credentials, query or fragment');
  if (value.provider === 'databricks_v2' && (url.pathname !== '/' || url.port)) throw Error('Databricks workspace requires an HTTPS origin without path or custom port');
  if (value.provider === 'openai-compat' ? !['auto', 'chat', 'responses'].includes(value.wire ?? '') : value.wire !== undefined) throw Error('Unsupported provider wire combination; only OpenAI-compatible accepts auto/chat/responses');
}
/** Closed launch environment; file presence is a prerequisite, never authentication proof. */
export function buzzProviderEnvironment(value: BuzzProvider, home: string, configDirectory: string, model: string): Record<string, string> {
  validateBuzzProvider(value);
  if (!value.models.includes(model)) throw Error('Unsupported Buzz Agent model');
  const base = { PATH: '/usr/bin:/bin', HOME: home, BUZZ_AGENT_CONFIG_DIR: configDirectory, BUZZ_AGENT_PROVIDER: value.provider, BUZZ_AGENT_MODEL: model };
  if (value.auth === 'external-oauth') return { ...base, DATABRICKS_HOST: new URL(value.baseUrl).origin }; // Native owner alone reads/refreshes OAuth after explicit launch.
  const apiKeyFile = value.apiKeyFile!;
  let key: string;
  try {
    const st = statSync(apiKeyFile);
    if (realpathSync(apiKeyFile) !== apiKeyFile || !st.isFile() || st.size > 16384 || (st.mode & 0o077) !== 0 || st.uid !== process.getuid?.()) throw Error();
    key = readFileSync(apiKeyFile, 'utf8').trim();
    if (!key || /[\s\0]/.test(key)) throw Error();
  } catch { throw Error('Buzz Agent API key prerequisite missing/unsafe locally; provision an owner-only key file as the service user. No login initiated'); }
  const contract = contracts[value.provider];
  return { ...base, [contract.key]: key, [contract.url]: value.provider === 'databricks_v2' ? new URL(value.baseUrl).origin : value.baseUrl, ...(value.wire ? { OPENAI_COMPAT_API: value.wire } : {}) };
}
/** Operator guidance; no automatic install, login, refresh or cache harvesting. */
export const buzzProviderGuidance = 'Buzz Agent provider binding: install buzz-agent locally as the dedicated service user. Anthropic uses ANTHROPIC_API_KEY; OpenAI-compatible uses OPENAI_COMPAT_API_KEY (NOT OPENAI_API_KEY), with explicit auto/chat/responses wire mode; OpenRouter uses OPENROUTER_API_KEY and Chat Completions only. Endpoint and owner-only key file stay local. Approved models are not an authenticated catalog. Start must prove the exact session model; Save never restarts. Databricks v2 uses DATABRICKS_HOST and either an owner-only DATABRICKS_TOKEN file or explicitly external native OAuth (no token fallback). Local configuration and key presence are not verified login, refresh or model access. No login or endpoint probe initiated.';
/** Shared normal/management wizard, not a generic remote environment form. */
export async function buzzProviderInput(ui: { question(prompt: string): Promise<string> }): Promise<BuzzProvider> {
  console.log(buzzProviderGuidance);
  const provider = await ui.question('Buzz Agent provider [anthropic / openai-compat / openrouter / databricks_v2]: ') as BuzzProvider['provider'];
  if (!Object.hasOwn(contracts, provider)) throw Error('Unsupported Buzz Agent provider');
  const auth = provider === 'databricks_v2' ? await ui.question('Databricks authentication [token / external-oauth]: ') as BuzzProvider['auth'] : undefined;
  if (provider === 'databricks_v2' && !['token', 'external-oauth'].includes(auth ?? '')) throw Error('Choose explicit Databricks auth; no fallback');
  const apiKeyFile = auth === 'external-oauth' ? undefined : await ui.question(`Absolute owner-only local ${contracts[provider].key} file (contents never relayed): `);
  const baseUrl = await ui.question('Provider HTTPS base URL (local only): ');
  const wire = provider === 'openai-compat' ? await ui.question('OpenAI-compatible wire [auto / chat / responses]: ') as BuzzProvider['wire'] : undefined;
  const models = (await ui.question('Operator-approved compatible exact Buzz Agent model IDs (comma-separated): ')).split(',').map(m => m.trim());
  const result = { provider, ...(apiKeyFile === undefined ? {} : { apiKeyFile }), ...(auth ? { auth } : {}), baseUrl, ...(wire ? { wire } : {}), models };
  validateBuzzProvider(result); return result;
}

/** Manual native command only; never inspect another user's cache or execute auth here. */
export function databricksOAuthGuidance(runner: string, home: string, configDirectory: string, host: string): string {
  const quote = (s: string) => "'" + s.replaceAll("'", "'\\''") + "'";
  return `External OAuth is locally configured, credentials UNCHECKED, readiness UNVERIFIED. As the actual host-service OS user (not Desktop/personal HOME), intentionally run:
env -i PATH=/usr/bin:/bin HOME=${quote(home)} BUZZ_AGENT_CONFIG_DIR=${quote(configDirectory)} DATABRICKS_HOST=${quote(host)} ${quote(runner)} auth databricks_v2
Native login needs a browser on that machine. Native cache: BUZZ_AGENT_CONFIG_DIR/buzz-agent/oauth/databricks (default without override: ~/.config/buzz-agent/oauth/databricks), keyed by workspace discovery URL/client/scopes. Do not copy personal caches. No login, refresh, cache read or provider probe initiated. Missing credential/refresh rejection requires intentional native service-user sign-in; denied/network/timeout are not proof of missing credentials. Later explicit Start/Restart may invoke native headless refresh and must independently prove the exact session model/profile. Save is not Restart.`;
}
