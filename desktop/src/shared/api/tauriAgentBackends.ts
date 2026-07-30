/**
 * Agent backend surface: where an agent's harness runs, and the API for the
 * backends Buzz does not run in-process. Split from `tauri.ts`/`types.ts`
 * (file-size guard); re-exported from both so import sites are unchanged.
 */
import { invokeTauri } from "@/shared/api/tauri";

export type BackendProviderCandidate = {
  id: string;
  binaryPath: string;
};

export type BackendProviderProbeResult = {
  ok: boolean;
  name?: string;
  version?: string;
  description?: string;
  config_schema?: Record<string, unknown>;
};

/**
 * Env for an `external`-backend agent's container. Contains the agent's nsec —
 * never cache, log, or persist it.
 */
export type ExternalAgentEnv = {
  /** Sorted `KEY -> value` pairs, for display. */
  env: Record<string, string>;
  /** The same pairs as `KEY=value` lines, for `docker run --env-file`. */
  envFile: string;
};

export async function discoverBackendProviders(): Promise<
  BackendProviderCandidate[]
> {
  return invokeTauri<BackendProviderCandidate[]>("discover_backend_providers");
}

export async function probeBackendProvider(
  binaryPath: string,
): Promise<BackendProviderProbeResult> {
  return invokeTauri<BackendProviderProbeResult>("probe_backend_provider", {
    binaryPath,
  });
}

/**
 * Fetch the env block an `external`-backend agent's `buzz-acp` needs.
 *
 * The response contains the agent's nsec. Callers must not cache it (no
 * `useQuery`) and must clear it from component state when hidden — see
 * `ExternalAgentEnvBlock`. Rejects for any other backend.
 */
export async function getExternalAgentEnv(
  pubkey: string,
): Promise<ExternalAgentEnv> {
  return invokeTauri<ExternalAgentEnv>("get_external_agent_env", { pubkey });
}
