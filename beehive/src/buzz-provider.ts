import { readFileSync, realpathSync, statSync } from 'node:fs';
import { isAbsolute } from 'node:path';

/** Local immutable Buzz Agent API-key contracts, pinned config.rs:631–691,922–960. */
export type BuzzProvider = { provider: 'anthropic' | 'openai-compat' | 'openrouter'; apiKeyFile: string; baseUrl: string; models: string[]; wire?: 'auto' | 'chat' | 'responses' };
const contracts = {
  anthropic: { key: 'ANTHROPIC_API_KEY', url: 'ANTHROPIC_BASE_URL' },
  'openai-compat': { key: 'OPENAI_COMPAT_API_KEY', url: 'OPENAI_COMPAT_BASE_URL' },
  openrouter: { key: 'OPENROUTER_API_KEY', url: 'OPENROUTER_BASE_URL' },
} as const;
/** Validate without reading credentials, contacting endpoints or initiating login. */
export function validateBuzzProvider(value: BuzzProvider): void {
  if (!value || !Object.hasOwn(contracts, value.provider) || Object.keys(value).some(k => !['provider', 'apiKeyFile', 'baseUrl', 'models', 'wire'].includes(k)) || typeof value.apiKeyFile !== 'string' || !isAbsolute(value.apiKeyFile) || !Array.isArray(value.models) || !value.models.length || value.models.length > 100 || value.models.some(m => typeof m !== 'string' || !/^[a-zA-Z0-9_.:/-]{1,200}$/.test(m))) throw Error('Invalid local Buzz Agent provider/key-file/models binding');
  let url: URL;
  try { url = new URL(value.baseUrl); } catch { throw Error('Invalid local provider endpoint'); }
  if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash) throw Error('Provider endpoint requires HTTPS without credentials, query or fragment');
  if (value.provider === 'openai-compat' ? !['auto', 'chat', 'responses'].includes(value.wire ?? '') : value.wire !== undefined) throw Error('Unsupported provider wire combination; only OpenAI-compatible accepts auto/chat/responses');
}
/** Closed launch environment; file presence is a prerequisite, never authentication proof. */
export function buzzProviderEnvironment(value: BuzzProvider, home: string, configDirectory: string, model: string): Record<string, string> {
  validateBuzzProvider(value);
  if (!value.models.includes(model)) throw Error('Unsupported Buzz Agent model');
  let key: string;
  try {
    const st = statSync(value.apiKeyFile);
    if (realpathSync(value.apiKeyFile) !== value.apiKeyFile || !st.isFile() || st.size > 16384 || (st.mode & 0o077) !== 0 || st.uid !== process.getuid?.()) throw Error();
    key = readFileSync(value.apiKeyFile, 'utf8').trim();
    if (!key || /[\s\0]/.test(key)) throw Error();
  } catch { throw Error('Buzz Agent API key prerequisite missing/unsafe locally; provision an owner-only key file as the service user. No login initiated'); }
  const contract = contracts[value.provider];
  return { PATH: '/usr/bin:/bin', HOME: home, BUZZ_AGENT_CONFIG_DIR: configDirectory, BUZZ_AGENT_PROVIDER: value.provider, BUZZ_AGENT_MODEL: model, [contract.key]: key, [contract.url]: value.baseUrl, ...(value.wire ? { OPENAI_COMPAT_API: value.wire } : {}) };
}
/** Operator guidance; no automatic install, login, refresh or cache harvesting. */
export const buzzProviderGuidance = 'Buzz Agent API-key binding: install buzz-agent locally as the dedicated service user. Anthropic uses ANTHROPIC_API_KEY; OpenAI-compatible uses OPENAI_COMPAT_API_KEY (NOT OPENAI_API_KEY), with explicit auto/chat/responses wire mode; OpenRouter uses OPENROUTER_API_KEY and Chat Completions only. Endpoint and owner-only key file stay local. Approved models are not an authenticated catalog. Start must prove the exact session model; Save never restarts. No login or endpoint probe initiated.';
/** Shared normal/management wizard, not a generic remote environment form. */
export async function buzzProviderInput(ui: { question(prompt: string): Promise<string> }): Promise<BuzzProvider> {
  console.log(buzzProviderGuidance);
  const provider = await ui.question('Buzz Agent provider [anthropic / openai-compat / openrouter]: ') as BuzzProvider['provider'];
  if (!Object.hasOwn(contracts, provider)) throw Error('Unsupported Buzz Agent provider');
  const apiKeyFile = await ui.question(`Absolute owner-only local ${contracts[provider].key} file (contents never relayed): `);
  const baseUrl = await ui.question('Provider HTTPS base URL (local only): ');
  const wire = provider === 'openai-compat' ? await ui.question('OpenAI-compatible wire [auto / chat / responses]: ') as BuzzProvider['wire'] : undefined;
  const models = (await ui.question('Operator-approved compatible exact Buzz Agent model IDs (comma-separated): ')).split(',').map(m => m.trim());
  const result = { provider, apiKeyFile, baseUrl, ...(wire ? { wire } : {}), models };
  validateBuzzProvider(result); return result;
}
