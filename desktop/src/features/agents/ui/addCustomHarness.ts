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
 *
 * `active` is the owning dialog's open state. The wait is only meaningful
 * while that dialog is open: both host dialogs stay mounted across closes, so
 * a pending id would otherwise survive the close and select into reset — or
 * hidden — form state whenever discovery caught up. Going inactive both blocks
 * `onReady` and drops the pending id, so a later publish is a no-op and
 * reopening starts clean. A second save before the first publishes replaces
 * it: the field holds one harness, so the latest save wins.
 */
export function usePendingHarnessSelection(
  runtimes: ReadonlyArray<{ id: string }>,
  onReady: (id: string) => void,
  active: boolean,
): (id: string) => void {
  const [pendingId, setPendingId] = React.useState<string | null>(null);
  // Gated at render, not just in the effect, so a catalog update landing in
  // the same commit as the close cannot slip a selection through.
  const readyId = active ? readyHarnessId(runtimes, pendingId) : null;

  React.useEffect(() => {
    if (!active) {
      setPendingId(null);
      return;
    }
    if (readyId === null) return;
    setPendingId(null);
    onReady(readyId);
  }, [active, onReady, readyId]);

  return setPendingId;
}
