import type {
  AcpRuntime,
  AcpRuntimeCatalogEntry,
  AgentPersona,
  CreateManagedAgentInput,
} from "@/shared/api/types";
import {
  getDefaultPersonaRuntime,
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
  // Use the bundled-Goose-first preference (goose → buzz-agent → first available)
  // so runtime-less personas consistently pick the bundled default.
  const defaultRuntime = getDefaultPersonaRuntime(runtimes);
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
 * Where the started instance should run when the user picked something other
 * than plain local in the definition-create flow (B5). Absent intent =
 * today's local mapping, byte-identical.
 *
 * - `provider`: remote backend. Mirrors the legacy provider-mode create:
 *   no local ACP/agent/MCP commands are spawned, so none are set;
 *   `startOnAppLaunch` is forced false (remote agents don't auto-start with
 *   the desktop) and `spawnAfterCreate` true.
 * - `mesh`: relay-mesh compute. The preset patch carries the instance
 *   commands/env the legacy dialog fanned into its field state; env lands in
 *   record env_vars (the instance-override layer — the dial pointer is
 *   per-instance runtime state, never definition env). `harnessOverride`
 *   is true because the preset commands deliberately override the
 *   definition's runtime preference.
 */
export type BackendIntent = {
  type: "provider";
  id: string;
  config: Record<string, unknown>;
};

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
 *   later definition edits made before the first spawn. (Mesh preset env is
 *   the deliberate exception: it is instance-override state, not
 *   definition env.)
 */
export async function buildInstanceInputForDefinition(
  persona: AgentPersona,
  runtime: AcpRuntime,
  upload?: UploadMediaBytes,
  backendIntent?: BackendIntent,
): Promise<CreateManagedAgentInput> {
  const avatarUrl = await resolveManagedAgentAvatarUrl(
    persona.avatarUrl,
    upload,
    runtime.avatarUrl,
  );

  const base = {
    name: persona.displayName,
    personaId: persona.id,
    systemPrompt: persona.systemPrompt,
    avatarUrl,
  };

  if (backendIntent?.type === "provider") {
    return {
      ...base,
      harnessOverride: false,
      spawnAfterCreate: true,
      startOnAppLaunch: false,
      backend: {
        type: "provider",
        id: backendIntent.id,
        config: backendIntent.config,
      },
    };
  }

  return {
    ...base,
    acpCommand: "buzz-acp",
    agentCommand: runtime.command,
    agentArgs: runtime.defaultArgs,
    mcpCommand: runtime.mcpCommand ?? "",
    harnessOverride: !persona.runtime || persona.runtime === runtime.id,
    model: persona.model ?? undefined,
    provider: persona.provider ?? undefined,
    spawnAfterCreate: true,
    startOnAppLaunch: true,
    backend: { type: "local" },
  };
}
