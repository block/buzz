/**
 * Pure logic for the terse-register experiment toggle.
 *
 * The toggle's source of truth is the global agent config's env_vars —
 * NOT the preview-features localStorage store — because the behavior is
 * enacted by the `BUZZ_ACP_TERSE_REGISTER` env var baked into every managed
 * agent at spawn. Deriving the switch state from the same file that controls
 * the behavior means the UI can never drift from what agents are actually
 * doing (a localStorage-backed flag could show "off" while a stale env var
 * kept agents terse).
 *
 * Enable = set the env var to "true" (the value shape `buzz-acp`'s clap
 * bool parsing expects, mirroring BUZZ_ACP_LAZY_POOL). Disable = remove the
 * key entirely, matching the global config's "absent = inherit default"
 * semantics; the harness default is off.
 */

export const TERSE_REGISTER_ENV_KEY = "BUZZ_ACP_TERSE_REGISTER";

/** Minimal structural slice of GlobalAgentConfig this logic needs. */
export type TerseRegisterEnvConfig = {
  env_vars: Record<string, string>;
};

/** True when the global config enables terse register. */
export function isTerseRegisterEnabled(
  config: TerseRegisterEnvConfig,
): boolean {
  return config.env_vars[TERSE_REGISTER_ENV_KEY] === "true";
}

/**
 * Return a copy of `config` with terse register enabled or disabled.
 *
 * Never mutates the input. Disabling removes the key (rather than writing
 * "false") so an explicit user-provided "false" and "toggled off" converge
 * on the same clean state.
 */
export function withTerseRegister<T extends TerseRegisterEnvConfig>(
  config: T,
  enabled: boolean,
): T {
  const env_vars = { ...config.env_vars };
  if (enabled) {
    env_vars[TERSE_REGISTER_ENV_KEY] = "true";
  } else {
    delete env_vars[TERSE_REGISTER_ENV_KEY];
  }
  return { ...config, env_vars };
}
