import {
  createPersonaDialogState,
  type PersonaDialogState,
} from "./personaDialogState";

/**
 * What the user is creating from the unified create dialog.
 *
 * - `definition` — a keyless agent definition (persona record) only.
 * - `definition_start` — definition plus an immediately created + spawned
 *   managed instance linked via `personaId` (today's quick-start flow).
 * - `instance` — a standalone keyed managed agent (no definition record).
 */
export type AgentCreateIntent = "definition" | "definition_start" | "instance";

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

/** Maps the "Start agent after create" toggle to a definition-family intent. */
export function intentForStartToggle(
  startAfterCreate: boolean,
): AgentCreateIntent {
  return startAfterCreate ? "definition_start" : "definition";
}

/**
 * Dialog copy for the definition-family create dialog. The toggle-on copy is
 * derived from `createPersonaDialogState` so it cannot drift from the legacy
 * create flow it replaces.
 */
export function definitionCreateDialogState(
  startAfterCreate: boolean,
): PersonaDialogState {
  const legacy = createPersonaDialogState();
  if (startAfterCreate) {
    return legacy;
  }

  return {
    ...legacy,
    description:
      "Create an agent profile without starting an instance. You can start it from its card at any time.",
  };
}
