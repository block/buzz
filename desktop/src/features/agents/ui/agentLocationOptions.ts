import { LOCAL, type AgentLocation } from "../agentLocation";

/** `<select>` value that means "run this agent on this computer". */
export const LOCAL_LOCATION_VALUE = "local";

/** One entry of the create dialog's "Runs on" control. */
export type AgentLocationOption = {
  /** `LOCAL_LOCATION_VALUE`, or the spawner's pubkey. */
  value: string;
  label: string;
  /** Secondary detail — the runtime a spawner advertises, when it announced one. */
  hint?: string;
};

/**
 * Resolves display text for a spawner pubkey.
 *
 * Injected rather than imported so this module stays free of React and of the
 * spawner directory store: the labels come from an announcement that may not
 * have arrived, and the option list is otherwise pure.
 */
export type SpawnerDescriptor = (pubkey: string) => {
  label: string;
  hint?: string;
};

/**
 * The locations a new agent can be created in.
 *
 * Empty when no spawner is connected — there is exactly one possible answer in
 * that case, so the caller hides the control rather than rendering a one-item
 * dropdown.
 */
export function agentLocationOptions(
  spawners: readonly string[],
  describe: SpawnerDescriptor,
): AgentLocationOption[] {
  if (spawners.length === 0) return [];
  return [
    { value: LOCAL_LOCATION_VALUE, label: "This Mac" },
    ...spawners.map((pubkey) => {
      const { label, hint } = describe(pubkey);
      return { value: pubkey, label, ...(hint ? { hint } : {}) };
    }),
  ];
}

/** The `<select>` value for a location. */
export function agentLocationValue(location: AgentLocation): string {
  return location.kind === "local"
    ? LOCAL_LOCATION_VALUE
    : location.spawnerPubkey;
}

/**
 * The location a `<select>` value refers to.
 *
 * A value that is not a currently connected spawner resolves to local: a
 * spawner can disconnect while the dialog is open, and deploying to one this
 * device no longer manages would fail with no obvious cause.
 */
export function parseAgentLocationValue(
  value: string,
  spawners: readonly string[],
): AgentLocation {
  const pubkey = value.trim().toLowerCase();
  if (pubkey === LOCAL_LOCATION_VALUE) return LOCAL;
  return spawners.includes(pubkey)
    ? { kind: "spawner", spawnerPubkey: pubkey }
    : LOCAL;
}
