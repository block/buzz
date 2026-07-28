/**
 * Shared pieces of the inline "Add custom harness…" entry the agent dialogs
 * append to their harness dropdown.
 *
 * Registering a custom harness used to be reachable only from Settings, so
 * anyone whose first stop was "New agent" never learned the path existed.
 * These helpers keep the entry identical across the dropdowns, keep its
 * sentinel value out of form state, and defer selecting a freshly registered
 * harness until discovery has actually published it.
 */

import * as React from "react";

import {
  NO_RUNTIME_DROPDOWN_VALUE,
  type PersonaDropdownOption,
} from "./agentConfigOptions";

/**
 * Dropdown value for the add-custom-harness entry. NUL-prefixed so it can
 * never collide with a harness id (`[a-z0-9_][a-z0-9_-]*`) — same trick as the
 * harness catalog's `CUSTOM_ENTRY_ID`.
 */
export const ADD_CUSTOM_HARNESS_VALUE = "\u0000add-custom-harness";

export const ADD_CUSTOM_HARNESS_OPTION: PersonaDropdownOption = {
  label: "Add custom harness…",
  value: ADD_CUSTOM_HARNESS_VALUE,
};

export type RuntimeDropdownAction =
  | { kind: "add-custom-harness" }
  | { kind: "select"; runtimeId: string };

/**
 * Route a harness-dropdown change. The add-custom entry only opens the
 * registration form — it is never a selection, so its sentinel can't reach
 * form state. Every other value selects, with the no-runtime sentinel
 * normalized to the empty id.
 */
export function runtimeDropdownAction(value: string): RuntimeDropdownAction {
  if (value === ADD_CUSTOM_HARNESS_VALUE) {
    return { kind: "add-custom-harness" };
  }
  return {
    kind: "select",
    runtimeId: value === NO_RUNTIME_DROPDOWN_VALUE ? "" : value,
  };
}

/**
 * The pending harness id once discovery has published it, else `null`.
 *
 * Saving only writes the definition file — the harness becomes a catalog entry
 * when the invalidated discovery query refetches. Selecting before then would
 * pick an id no entry backs: the create dialog would block Save on an unknown
 * availability, and the instance dialog could not read the command to pin.
 */
export function readyHarnessId(
  runtimes: ReadonlyArray<{ id: string }>,
  pendingId: string | null,
): string | null {
  return runtimes.some((runtime) => runtime.id === pendingId)
    ? pendingId
    : null;
}

/**
 * Selects a newly registered custom harness once discovery publishes it.
 *
 * Returns the setter to hand the saved id; `onReady` then fires with it, so
 * callers reuse their normal dropdown-change path instead of growing a second
 * selection code path.
 */
export function usePendingHarnessSelection(
  runtimes: ReadonlyArray<{ id: string }>,
  onReady: (id: string) => void,
): (id: string) => void {
  const [pendingId, setPendingId] = React.useState<string | null>(null);
  const readyId = readyHarnessId(runtimes, pendingId);

  React.useEffect(() => {
    if (readyId === null) return;
    setPendingId(null);
    onReady(readyId);
  }, [onReady, readyId]);

  return setPendingId;
}
