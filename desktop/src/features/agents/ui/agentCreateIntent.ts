/**
 * What the user is creating from the unified create dialog.
 *
 * - `definition` — a keyless agent definition (persona record) only.
 * - `definition_start` — definition plus an immediately created + spawned
 *   managed instance linked via `personaId` (today's quick-start flow).
 */
export type AgentCreateIntent = "definition" | "definition_start";

/**
 * Default intent for callers that don't pass one. Un-migrated callers of
 * `usePersonaActions.handleSubmit` (AgentDefinitionDialog's duplicate path
 * until B3) must keep today's create-then-start semantics, so the default is
 * `definition_start`, never `definition`.
 */
export function resolveCreateIntent(
  intent?: AgentCreateIntent,
): AgentCreateIntent {
  return intent ?? "definition_start";
}
