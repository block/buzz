import { accessSync, constants, readFileSync, realpathSync, statSync } from 'node:fs';
import { dirname, isAbsolute } from 'node:path';

/** Pinned Buzz pool.rs:287–320: this identity supports native system prompt append. */
export const CLAUDE_ADAPTER = '@agentclientprotocol/claude-agent-acp';
/** Local-only Claude API-key binding. Subscription/Bedrock/Vertex modes are not inferred. */
export type ClaudeSetup = { cli: string; apiKeyFile: string; models: string[] };
/** Validate private inputs without reading a credential during offline setup. */
export function validateClaude(value: ClaudeSetup): void {
  if (!value || !isAbsolute(value.cli) || !isAbsolute(value.apiKeyFile) || !Array.isArray(value.models) || !value.models.length || value.models.length > 100 || value.models.some(m => typeof m !== 'string' || !/^[a-zA-Z0-9_.:/-]{1,200}$/.test(m))) throw Error('Invalid local Claude CLI/key-file/models binding');
}
/** Build a closed service-user env. A local key is a prerequisite, NOT authentication proof. */
export function claudeEnvironment(value: ClaudeSetup, home: string, model: string): Record<string, string> {
  validateClaude(value);
  try {
    if (realpathSync(value.cli) !== value.cli) throw Error();
    accessSync(value.cli, constants.X_OK);
  } catch { throw Error('Claude CLI missing/noncanonical; install Claude locally as the service user. No install initiated'); }
  let key: string;
  try {
    const st = statSync(value.apiKeyFile);
    if (realpathSync(value.apiKeyFile) !== value.apiKeyFile || !st.isFile() || st.size > 16384 || (st.mode & 0o077) !== 0 || st.uid !== process.getuid?.()) throw Error();
    key = readFileSync(value.apiKeyFile, 'utf8').trim();
    if (!key || /[\s\0]/.test(key)) throw Error();
  } catch { throw Error('Claude API key prerequisite missing/unsafe locally; provision an owner-only key file as the service user. No login initiated'); }
  return { PATH: `${dirname(value.cli)}:${dirname(process.execPath)}:/usr/bin:/bin`, HOME: home, CLAUDE_CODE_EXECUTABLE: value.cli, ANTHROPIC_API_KEY: key, ANTHROPIC_MODEL: model };
}
/** Positive same-session startup model report, never a fabricated set_model RPC ACK. */
export function claudeModels(value: any, model: string): string[] {
  const models = value?.models;
  if (models?.currentModelId !== model || !Array.isArray(models.availableModels) || models.availableModels.length > 1000) throw Error('Claude exact startup model evidence unavailable/mismatched; adapter capability gap');
  const ids = models.availableModels.map((m: any) => m?.modelId);
  if (ids.some((id: unknown) => typeof id !== 'string' || !/^[a-zA-Z0-9_.:/-]{1,200}$/.test(id)) || !ids.includes(model)) throw Error('Claude exact model not advertised');
  return ids;
}
/** Local recovery guidance only; no install/login/status command is executed. */
export const claudeGuidance = 'Claude Code requires the locally installed claude CLI and claude-agent-acp adapter (npm install -g @agentclientprotocol/claude-agent-acp). This binding supports ANTHROPIC_API_KEY from an owner-only local file, not a universal provider form. Claude owns ~/.claude/settings.json in the dedicated service HOME. Subscription login (Claude CLI; claude auth status) is source-known but not enabled by this API-key binding. No Goose/Buzz OAuth cache reuse; executable/key presence is not authenticated. Model/profile changes require Restart.';
