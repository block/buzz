import type {
  AcpRuntime,
  AcpRuntimeCatalogEntry,
  AgentPersona,
  CreateManagedAgentInput,
} from "@/shared/api/types";
import {
  resolvePersonaRuntime,
  type ResolvePersonaRuntimeResult,
} from "./resolvePersonaRuntime";
import {
  resolveManagedAgentAvatarUrl,
  type UploadMediaBytes,
} from "../ui/managedAgentAvatar";

type RuntimesQueryLike = {
  isFetched: boolean;
  data: readonly AcpRuntimeCatalogEntry[] | undefined;
  refetch: () => Promise<{
    data?: readonly AcpRuntimeCatalogEntry[] | undefined;
  }>;
};

/**
 * Acquire the available-runtime list for a start action (Phase 1B.3.5
 * row 6). Refetch-aware: an unfetched query is fetched instead of being
 * treated as an empty list (which would spuriously refuse every start).
 */
export async function availableRuntimesForStart(
  query: RuntimesQueryLike,
): Promise<AcpRuntime[]> {
  const entries = query.isFetched ? query.data : (await query.refetch()).data;
  return (entries ?? []).filter(
    (runtime): runtime is AcpRuntime => runtime.availability === "available",
  );
}

/**
 * Resolve the runtime a definition should start on, refusing when the
 * definition's configured runtime is not available (Phase 1B.3.5 row 1,
 * Wes's call: one consistent refuse-with-actionable-error everywhere —
 * never silently start on a different runtime than configured).
 */
export function resolveStartRuntimeForDefinition(
  persona: AgentPersona,
  runtimes: readonly AcpRuntime[],
): { runtime: AcpRuntime; warnings: string[] } {
  const defaultRuntime = runtimes[0] ?? null;
  const { runtime, warnings, isOverridden }: ResolvePersonaRuntimeResult =
    resolvePersonaRuntime(persona.runtime, runtimes, defaultRuntime);

  if (!runtime) {
    throw new Error("No available runtime found for this agent.");
  }
  if (isOverridden) {
    throw new Error(
      warnings[0] ??
        "This agent's configured runtime is not available. Install the runtime or edit the agent before starting it.",
    );
  }
  return { runtime, warnings };
}

/**
 * The single definition→instance mapping (Phase 1B.3.5 rows 2–4). Every
 * surface that creates a running instance from a definition builds its
 * CreateManagedAgentInput here so the mapping cannot drift per-site.
 *
 * - harnessOverride uses the backend-aligned formula: true only when the
 *   definition has no runtime preference or the picked runtime matches it
 *   (`create_time_agent_command_override` stores None when picked ==
 *   inherited; on fallback `harness_override: false` keeps the definition
 *   authoritative).
 * - avatarUrl goes through resolveManagedAgentAvatarUrl (base64 data URIs
 *   upload via the injectable `upload`; other URLs pass through unchanged).
 * - envVars are never seeded from the definition: record.env_vars is
 *   agent overrides only and spawn merges the live definition env
 *   underneath. Seeding would manufacture pseudo-overrides that mask
 *   later definition edits made before the first spawn.
 */
export async function buildInstanceInputForDefinition(
  persona: AgentPersona,
  runtime: AcpRuntime,
  upload?: UploadMediaBytes,
): Promise<CreateManagedAgentInput> {
  const avatarUrl = await resolveManagedAgentAvatarUrl(
    persona.avatarUrl,
    upload,
    runtime.avatarUrl,
  );

  return {
    name: persona.displayName,
    acpCommand: "buzz-acp",
    agentCommand: runtime.command,
    agentArgs: runtime.defaultArgs,
    mcpCommand: runtime.mcpCommand ?? "",
    personaId: persona.id,
    harnessOverride: !persona.runtime || persona.runtime === runtime.id,
    systemPrompt: persona.systemPrompt,
    avatarUrl,
    model: persona.model ?? undefined,
    spawnAfterCreate: true,
    startOnAppLaunch: true,
    backend: { type: "local" },
  };
}
