import type { BackendIntent } from "../lib/instanceInputForDefinition";
import type { BackendProviderProbeResult } from "@/shared/api/types";
import { coerceConfigValues } from "./ProviderConfigFields";

/** Draft state of the optional remote-backend selector. */
export type WhereToRunDraft = {
  runOn: "local" | string;
  providerConfig: Record<string, string>;
  probedProvider: BackendProviderProbeResult | null;
};

export const emptyWhereToRunDraft: WhereToRunDraft = {
  runOn: "local",
  providerConfig: {},
  probedProvider: null,
};

/**
 * Merge a completed provider probe into the draft. Seeds schema defaults for
 * config keys the user hasn't set yet, but preserves any value the user has
 * already entered.
 *
 * This ordering matters: the probe is async, so it can resolve *after* the user
 * has started typing. Overwriting `providerConfig` with the raw defaults there
 * wiped the input mid-typing (e.g. the Blox `workstation_name` field resetting
 * itself). Defaults are spread first so user-entered values win.
 */
export function applyProbeResult(
  draft: WhereToRunDraft,
  result: BackendProviderProbeResult,
): WhereToRunDraft {
  const defaults: Record<string, string> = {};
  const schema = result.config_schema as Record<string, unknown> | undefined;
  const properties =
    (schema?.properties as
      | Record<string, Record<string, unknown>>
      | undefined) ?? {};
  for (const [key, property] of Object.entries(properties)) {
    if (property.default != null) defaults[key] = String(property.default);
  }
  return {
    ...draft,
    probedProvider: result,
    providerConfig: { ...defaults, ...draft.providerConfig },
  };
}

export function providerConfigComplete(draft: WhereToRunDraft): boolean {
  if (draft.runOn === "local") return true;
  if (!draft.probedProvider) return false;
  const schema = draft.probedProvider.config_schema as
    | Record<string, unknown>
    | undefined;
  const required: string[] = (schema?.required as string[] | undefined) ?? [];
  return required.every(
    (key) => (draft.providerConfig[key] ?? "").trim().length > 0,
  );
}

export function canSubmitWhereToRun(draft: WhereToRunDraft): boolean {
  return providerConfigComplete(draft);
}

export function resolveBackendIntent(
  draft: WhereToRunDraft,
): BackendIntent | null {
  if (draft.runOn === "local") return null;
  return {
    type: "provider",
    id: draft.runOn,
    config: coerceConfigValues(
      draft.providerConfig,
      draft.probedProvider?.config_schema,
    ),
  };
}
