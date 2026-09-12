import { accessSync, constants, readFileSync, realpathSync, statSync } from 'node:fs';
import type { ProviderReference } from './settings.ts';
import { dirname, isAbsolute } from 'node:path';

/** Pinned Buzz 051c3a2 pool.rs:295–320: Codex native systemPrompt requires protocol 2. */
export const CODEX_ADAPTER = 'codex-acp';
/** Local-only Codex API-key binding. ChatGPT subscription login caches and custom providers are not inferred. */
export type CodexSetup = { cli: string; apiKeyFile?: string; credential?: ProviderReference; models: string[] };
/** Validate private inputs without reading a credential during offline setup. */
export function validateCodex(value: CodexSetup): void {
  if (!value || !isAbsolute(value.cli) || (value.credential ? value.apiKeyFile !== undefined || value.credential.service !== 'beehive' || !/^provider:[0-9a-f-]{36}$/.test(value.credential.account) : typeof value.apiKeyFile !== 'string' || !isAbsolute(value.apiKeyFile)) || !Array.isArray(value.models) || !value.models.length || value.models.length > 100 || value.models.some(m => typeof m !== 'string' || !/^[a-zA-Z0-9_.:/-]{1,200}$/.test(m))) throw Error('Invalid local Codex CLI/key-file/models binding');
}
/** Build a closed service-user env. A local key is a prerequisite, NOT authentication proof. */
export function codexEnvironment(value: CodexSetup, home: string, configDirectory: string, model: string, resolvedKey?: string): Record<string, string> {
  validateCodex(value);
  try {
    if (realpathSync(value.cli) !== value.cli) throw Error();
    accessSync(value.cli, constants.X_OK);
  } catch { throw Error('Codex CLI missing/noncanonical; install Codex locally as the service user. No install initiated'); }
  let key: string;
  try {
    if (value.credential) {
      if (!resolvedKey || resolvedKey.length > 16384 || /[\s\0]/.test(resolvedKey)) throw Error();
      key = resolvedKey;
    } else {
      const st = statSync(value.apiKeyFile!);
      if (realpathSync(value.apiKeyFile!) !== value.apiKeyFile || !st.isFile() || st.size > 16384 || (st.mode & 0o077) !== 0 || st.uid !== process.getuid?.()) throw Error();
      key = readFileSync(value.apiKeyFile!, 'utf8').trim();
      if (!key || /[\s\0]/.test(key)) throw Error();
    }
  } catch { throw Error(value.credential ? 'Codex OS provider credential unavailable; repair the saved OpenAI provider. No file fallback or login initiated' : 'Codex API key prerequisite missing/unsafe locally; provision an owner-only key file as the service user. No login initiated'); }
  return { PATH: `${dirname(value.cli)}:${dirname(process.execPath)}:/usr/bin:/bin`, HOME: home, CODEX_HOME: configDirectory, OPENAI_API_KEY: key, CODEX_CONFIG: JSON.stringify({ model }) };
}
/** Positive same-session startup model report, never a fabricated set_model RPC ACK. */
export function codexModels(value: any, model: string): string[] {
  const models = value?.models;
  if (models?.currentModelId !== model || !Array.isArray(models.availableModels) || models.availableModels.length > 1000) throw Error('Codex exact startup model evidence unavailable/mismatched; adapter capability gap');
  const ids = models.availableModels.map((m: any) => m?.modelId);
  if (ids.some((id: unknown) => typeof id !== 'string' || !/^[a-zA-Z0-9_.:/-]{1,200}$/.test(id)) || !ids.includes(model)) throw Error('Codex exact model not advertised');
  return ids;
}
/** Local recovery guidance only: no provider command or owner cache is accessed. */
export const codexGuidance = 'Codex requires separately installed codex and codex-acp. This closed binding supports only an explicit owner-only OPENAI_API_KEY file, not ChatGPT login-cache parity. Configure as the dedicated service OS user: HOME and CODEX_HOME are explicit; Codex reads CODEX_HOME/config.toml (TOML). CODEX_CONFIG is a JSON session override containing only the selected model, NOT a file path. codex login and codex login status are operator guidance only, never run here; login caches are not imported. Executable/key presence is not authenticated. ACP protocol 2 native systemPrompt and fresh exact model report are required before prompts; unsupported adapters fail closed. No install/login, Goose methods or Claude append. Changes require explicit Restart.';
