import { requiredCredentialEnvKeys } from "@/features/agents/ui/personaDialogPickers";
import type {
  AcpRuntimeCatalogEntry,
  GlobalAgentConfig,
} from "@/shared/api/types";

export type AgentReadinessResult =
  | { ready: true; reason: "cli"; runtimeLabel: string }
  | { ready: true; reason: "buzz-agent" }
  | { ready: false };

/**
 * Determine whether the user has a working agent path configured.
 *
 * CLI path: at least one non-buzz-agent runtime is available and logged in.
 * buzz-agent path: provider and model are set, and all required credential
 * env vars for that provider are present.
 *
 * Returns enough info for the UI to say which path matched, or that neither did.
 */
export function resolveAgentReadiness(
  runtimes: readonly AcpRuntimeCatalogEntry[],
  globalConfig: GlobalAgentConfig,
): AgentReadinessResult {
  // CLI path — any non-buzz-agent runtime that is available and has an
  // auth status indicating it can run: logged_in for runtimes that require
  // auth (Claude Code), or not_applicable for login-free harnesses (Goose).
  for (const runtime of runtimes) {
    if (runtime.id === "buzz-agent") continue;
    if (
      runtime.availability === "available" &&
      (runtime.authStatus.status === "logged_in" ||
        runtime.authStatus.status === "not_applicable")
    ) {
      return { ready: true, reason: "cli", runtimeLabel: runtime.label };
    }
  }

  // buzz-agent path — provider + model + required credential keys all present.
  const provider = globalConfig.provider?.trim() ?? "";
  const model = globalConfig.model?.trim() ?? "";
  if (provider.length > 0 && model.length > 0) {
    const required = requiredCredentialEnvKeys("buzz-agent", provider);
    const allKeysPresent = required.every(
      (key) => (globalConfig.env_vars[key] ?? "").trim().length > 0,
    );
    if (allKeysPresent) {
      return { ready: true, reason: "buzz-agent" };
    }
  }

  return { ready: false };
}
