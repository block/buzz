import { accessSync, constants, realpathSync } from 'node:fs';
import { dirname, isAbsolute } from 'node:path';
import { validateBuzzProvider, type BuzzProvider } from './buzz-provider.ts';
import { runtimeEfforts, runtimeCapabilities } from './runtime-effort.ts';

/** Desktop 78618804: fork identity (not upstream pi-acp) gates native metadata.
 * The fork has no Desktop semver floor; positive session config evidence is required. */
export const PI_ADAPTER = 'buzz-pi-acp';
export function piEfforts(provider: string, model: string) {
  return runtimeEfforts(provider, model).filter(e => ['minimal','low','medium','high','xhigh'].includes(e));
}
export function validatePi(cli: string, provider: BuzzProvider) {
  validateBuzzProvider(provider);
  if (!isAbsolute(cli) || !provider.credential || !['openai-compat','databricks_v2'].includes(provider.provider) || provider.apiKeyFile !== undefined) throw Error('Pi requires a saved OS provider and explicit CLI');
  if (provider.effort !== undefined && provider.models.some(m => !piEfforts(provider.provider, m).includes(provider.effort!))) throw Error('Unsupported Pi effort');
}
export function piEnvironment(cli: string, provider: BuzzProvider, home: string, model: string, secret?: string): Record<string,string> {
  validatePi(cli, provider);
  if (realpathSync(cli) !== cli || !provider.models.includes(model)) throw Error('Pi CLI/model changed');
  accessSync(cli, constants.X_OK);
  if (!secret || secret.length > 16384 || /[\s\0]/.test(secret)) throw Error('Pi OS provider admission required');
  return { PATH: `${dirname(cli)}:${dirname(process.execPath)}:/usr/bin:/bin`, HOME: home, PI_ACP_PI_COMMAND: cli,
    BEEHIVE_PI_KEY: provider.provider === 'databricks_v2' ? 'native-refresh-owned' : secret,
    BEEHIVE_PI_RUNTIME: JSON.stringify({ provider, model }) };
}
/** Public-only models.json: apiKey names an environment variable, never a token.
 * Provider name is private to this run; raw model/FQN remains unchanged on wire. */
export function piModelConfig(provider: BuzzProvider, model: string, endpoint = provider.baseUrl) {
  const db = provider.provider === 'databricks_v2';
  const route = runtimeCapabilities(provider.provider,model)?.databricks_v2_wire_route;
  const anthropic = db && route === 'anthropic-messages';
  const responses = db && route === 'openai-responses';
  const api = anthropic ? 'anthropic-messages' : responses || !db ? 'openai-responses' : 'openai-completions';
  const baseUrl = endpoint + (db ? anthropic ? '/ai-gateway/anthropic' : responses ? '/ai-gateway/openai/v1' : '/ai-gateway/mlflow/v1' : '');
  return { providers: { beehive: { baseUrl, api, apiKey: '$BEEHIVE_PI_KEY', authHeader: true, models: [{ id: model, name: model, reasoning: piEfforts(provider.provider,model).length > 0, input: ['text'] }] } } };
}
export function piConfigEvidence(value: any, model: string, effort?: string): string[] {
  const options = value?.configOptions;
  if (!Array.isArray(options) || options.length > 128) throw Error('Pi config capability missing');
  for (const [category, expected] of [['model', `beehive/${model}`], ['thought_level', effort ?? 'off']]) {
    const matches = options.filter((o: any) => o.category === category);
    if (matches.length !== 1 || matches[0].currentValue !== expected || !Array.isArray(matches[0].options) || !matches[0].options.some((o: any) => o.value === expected)) throw Error('Pi exact model/effort evidence mismatch');
  }
  return [model];
}
