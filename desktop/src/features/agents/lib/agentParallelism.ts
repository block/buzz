/**
 * Desktop-managed agents materialize this value when neither the create input
 * nor the linked definition sets parallelism. Keep in sync with
 * `managed_agents::DEFAULT_AGENT_PARALLELISM` in the Tauri backend.
 */
export const DEFAULT_AGENT_PARALLELISM = 10;

export const AGENT_PARALLELISM_PLACEHOLDER = `App default (${DEFAULT_AGENT_PARALLELISM})`;
export const AGENT_PARALLELISM_HELP = `Leave blank to use the app default (currently ${DEFAULT_AGENT_PARALLELISM}). Custom values may be 1–32.`;
export const EDIT_AGENT_PARALLELISM_HELP =
  "Current value for this agent. Custom values may be 1–32.";

export function resolveAgentParallelism(
  input: number | undefined,
  definition: number | null | undefined,
): number {
  return input ?? definition ?? DEFAULT_AGENT_PARALLELISM;
}
