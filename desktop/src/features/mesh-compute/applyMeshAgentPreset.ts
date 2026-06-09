import type { MeshAgentPreset } from "@/shared/api/tauriMesh";

/**
 * Fields a Create-Agent draft needs to overwrite when the "Run on relay mesh"
 * flow is chosen. Mirror of `MeshAgentPreset` minus the picker metadata
 * (`providerId`, `label`) which the *flow* owns, not the agent record.
 */
export type MeshAgentPresetPatch = {
  acpCommand: string;
  agentCommand: string;
  agentArgs: string[];
  mcpCommand: string;
  model: string;
  envVars: Record<string, string>;
};

/**
 * Turn a `mesh_agent_preset()` response into the fields a managed-agent draft
 * should carry. Idempotent and contract-stable: the same preset always yields
 * the same patch. The flow integrator calls this and `Object.assign`s the
 * result over the draft fields.
 *
 * Why a separate function: the dialog's state lives across many `useState`s
 * (acpCommand, agentCommand, agentArgs, mcpCommand, model, envVars), so the
 * caller fans out the patch. Doing the fan-out via a single helper keeps the
 * integration commit a one-liner per setter and the override behavior
 * testable without rendering.
 */
export function meshAgentPresetPatch(
  preset: MeshAgentPreset,
): MeshAgentPresetPatch {
  return {
    acpCommand: preset.acpCommand,
    agentCommand: preset.agentCommand,
    agentArgs: [...preset.agentArgs],
    mcpCommand: preset.mcpCommand,
    model: preset.model,
    envVars: { ...preset.envVars },
  };
}

/**
 * Detect whether applying a Relay-mesh preset would overwrite values a user
 * (or persona) has already set. Drives the "Using relay mesh — overrides this
 * persona's model" honest-over-silent copy that Eva named as a requirement.
 *
 * Returns the list of human-readable field labels that the preset *changes*
 * from the current draft state, so the UI can render
 *   "Using Relay mesh overrides: model, agent runtime"
 * rather than a silent overwrite.
 *
 * Empty list = no overrides; the preset is purely additive.
 */
export function detectMeshPresetOverrides(
  current: {
    acpCommand: string;
    agentCommand: string;
    agentArgs: string[];
    mcpCommand: string;
    model: string | null;
    envVars: Record<string, string>;
  },
  preset: MeshAgentPreset,
): string[] {
  const changes: string[] = [];
  if (
    current.agentCommand !== "" &&
    current.agentCommand !== preset.agentCommand
  ) {
    changes.push("agent runtime");
  }
  if (current.acpCommand !== "" && current.acpCommand !== preset.acpCommand) {
    changes.push("ACP harness");
  }
  if (current.mcpCommand !== "" && current.mcpCommand !== preset.mcpCommand) {
    changes.push("MCP server");
  }
  if (
    current.model != null &&
    current.model.length > 0 &&
    current.model !== preset.model
  ) {
    changes.push("model");
  }
  // Env-var overlap: only mention if a *value* would change (a new key being
  // added is additive, not an override).
  const overlappingEnvKeys = Object.keys(preset.envVars).filter(
    (key) =>
      key in current.envVars && current.envVars[key] !== preset.envVars[key],
  );
  if (overlappingEnvKeys.length > 0) {
    changes.push("environment variables");
  }
  return changes;
}
