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
  preferredRuntimeId?: string | null,
): { runtime: AcpRuntime; warnings: string[] } {
  // Use the buzz-agent-first preference (buzz-agent → goose → first available)
  // so a freshly installed goose never beats the bundled buzz-agent sidecar
  // for runtime-less personas (item 13 regression guard).
  const defaultRuntime = getDefaultPersonaRuntime(runtimes, preferredRuntimeId);
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
 * Resolve the runtime for a definition CREATE, honouring where the agent will
 * actually run.
 *
 * For a local create the definition's runtime must be installed on this
 * machine, so an unavailable pick is refused exactly as before.
 *
 * For a provider create the harness lives on the REMOTE host and is chosen
 * from that host's catalog (`discover_provider_harnesses`); the local catalog
 * describes a different machine entirely. Requiring a locally-installed
 * runtime here would make every remote-only harness unsubmittable — the user
 * could not create a Goose agent on their server without first installing
 * Goose on their laptop — so the local availability check does not apply.
 * The runtime is still returned when it happens to resolve locally, because
 * callers use it for the avatar fallback; `null` simply means "no local
 * counterpart", which is normal and not an error.
 *
 * Returns `{ runtime }` on success and throws with an actionable message on
 * refusal, matching the start-path contract above.
 */
export function resolveCreateRuntimeForDefinition(
  runtimes: readonly AcpRuntime[],
  requestedRuntimeId: string | null | undefined,
  isProviderCreate: boolean,
): { runtime: AcpRuntime | null } {
  const runtime =
    runtimes.find((candidate) => candidate.id === requestedRuntimeId) ?? null;
  if (!runtime && !isProviderCreate) {
    throw new Error("Choose an available runtime for this agent.");
  }
  return { runtime };
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
  /**
   * The harness chosen from the REMOTE host's catalog
   * (`discover_provider_harnesses`). Correction C1: this is the only channel
   * by which the harness choice reaches the host, so it must name a binary on
   * the remote machine — never one resolved from the local runtime catalog.
   */
  harness?: RemoteHarnessSelection;
};

/**
 * One entry of a provider's `discover_harnesses` catalog, narrowed to the
 * fields the create mapping pins. `command` and `args` describe the remote
 * host; `env` carries the runtime's `default_env`, which locally would be
 * applied from the catalog at spawn time — a remote agent never spawns
 * locally, so it has to ride along in the record or it is simply lost.
 */
export type RemoteHarnessSelection = {
  id: string;
  command: string;
  args?: readonly string[];
  env?: Record<string, string>;
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
 *
 * The provider branch deliberately diverges from the local one on the harness
 * fields (command/args/env pinned rather than re-resolved). See the comments
 * inside it: nothing re-resolves them for a record that never spawns locally.
 */
export async function buildInstanceInputForDefinition(
  persona: AgentPersona,
  // Nullable only for a provider create: the harness then comes from the
  // remote catalog via `backendIntent.harness`, and no local runtime need
  // exist. The local branch below still requires one.
  runtime: AcpRuntime | null,
  upload?: UploadMediaBytes,
  backendIntent?: BackendIntent,
): Promise<CreateManagedAgentInput> {
  const avatarUrl = await resolveManagedAgentAvatarUrl(
    persona.avatarUrl,
    upload,
  );

  const base = {
    name: persona.displayName,
    personaId: persona.id,
    systemPrompt: persona.systemPrompt,
    avatarUrl,
  };

  if (backendIntent?.type === "provider") {
    const harness = backendIntent.harness;
    if (!harness?.command.trim()) {
      // Correction C1, refused at the source. Without a remote harness pin the
      // record falls through `create_time_agent_command_override` (which
      // returns None for a persona-backed create with harnessOverride: false)
      // to `effective_agent_command`, which resolves against the LOCAL runtime
      // registry and ultimately `default_agent_command()` — so the deploy
      // payload would carry "buzz-agent" and the host would silently provision
      // a harness the user never chose. The provider refuses a blank pin, but
      // a wrong non-blank one it cannot detect, so the guard has to live here.
      throw new Error(
        "Select a harness installed on the remote host before creating this agent.",
      );
    }
    return {
      ...base,
      // The pin is only preserved when harnessOverride is true: with false,
      // the backend treats a divergent command as a missing-runtime fallback
      // and discards it. A remote harness choice is always a deliberate pin —
      // the definition's runtime names a LOCAL runtime id that says nothing
      // about the remote machine, so it is never authoritative here.
      harnessOverride: true,
      agentCommand: harness.command,
      // Unlike the local branch (which sends [] so spawn re-resolves args from
      // the definition on every start), a provider-backed record never spawns
      // locally. These args and env come from the remote catalog and have no
      // second chance to be resolved, so they are pinned at create time.
      agentArgs: [...(harness.args ?? [])],
      ...(harness.env && Object.keys(harness.env).length > 0
        ? { envVars: { ...harness.env } }
        : {}),
      model: persona.model ?? undefined,
      provider: persona.provider ?? undefined,
      spawnAfterCreate: true,
      startOnAppLaunch: false,
      backend: {
        type: "provider",
        id: backendIntent.id,
        config: backendIntent.config,
      },
    };
  }

  if (!runtime) {
    // Unreachable through the UI (`resolveCreateRuntimeForDefinition` only
    // returns null for a provider create, which returned above), but a local
    // create with no runtime has no harness to spawn — refuse rather than
    // build an input whose agentCommand is undefined.
    throw new Error("Choose an available runtime for this agent.");
  }

  return {
    ...base,
    acpCommand: "buzz-acp",
    agentCommand: runtime.command,
    // Do NOT seed agentArgs from runtime.defaultArgs: record.agent_args must
    // remain empty so spawn resolves args live from the definition on every
    // start.  Seeding here would freeze the args at create-time, silently
    // ignoring any later definition-arg edits (Thufir F5 / phase B-5).
    // envVars are intentionally never seeded for the same reason (see comment
    // at top of this function).
    agentArgs: [],
    mcpCommand: runtime.mcpCommand ?? "",
    harnessOverride: !persona.runtime || persona.runtime === runtime.id,
    model: persona.model ?? undefined,
    provider: persona.provider ?? undefined,
    spawnAfterCreate: true,
    startOnAppLaunch: true,
    backend: { type: "local" },
  };
}
