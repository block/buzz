import { buildRuntimeModelProviderPayload } from "./agentDefinitionSubmitPayload";

type DefinitionExecutionConfiguration = {
  runtime?: string | null;
  model?: string | null;
  provider?: string | null;
  envVars?: Record<string, string> | null;
};

/**
 * Existing definitions may carry an execution configuration that is no longer
 * ready on this machine (for example, a remote provider owns execution while
 * the local provider/model defaults are unset). Editing profile fields must
 * not force that unrelated configuration through today's local readiness
 * gate. Once any execution field changes, the normal readiness rules apply.
 */
export function definitionEditPreservesExecutionConfiguration({
  initial,
  runtime,
  model,
  provider,
  envVars,
  isRuntimeAutoSeeded,
}: {
  initial: DefinitionExecutionConfiguration;
  runtime: string;
  model: string;
  provider: string;
  envVars: Record<string, string>;
  isRuntimeAutoSeeded: boolean;
}): boolean {
  const initialRuntime = normalizeOptionalString(initial.runtime);
  const initialModel = normalizeOptionalString(initial.model);
  const initialProvider = normalizeOptionalString(initial.provider);
  const next = buildRuntimeModelProviderPayload({
    runtime,
    model,
    provider,
    isEditMode: true,
    isAutoSeeded: isRuntimeAutoSeeded,
    initialPreviousRuntime: initialRuntime,
    initialModel: initial.model,
    initialProvider: initial.provider,
    initialModelProviderEditableWithoutRuntime:
      initialRuntime.length === 0 &&
      (initialModel.length > 0 || initialProvider.length > 0),
  });

  return (
    normalizeOptionalString(next.runtime) === initialRuntime &&
    normalizeOptionalString(next.model) === initialModel &&
    normalizeOptionalString(next.provider) === initialProvider &&
    stringRecordsEqual(envVars, initial.envVars ?? {})
  );
}

/**
 * Readiness is mandatory for creates and execution-config edits. An existing
 * definition may still save profile/behavior changes when its execution
 * configuration's normalized submit projection is unchanged.
 */
export function definitionExecutionReadinessSatisfied({
  isEditMode,
  preservesExecutionConfiguration,
  localModeSatisfied,
  customAiPairSatisfied,
}: {
  isEditMode: boolean;
  preservesExecutionConfiguration: boolean;
  localModeSatisfied: boolean;
  customAiPairSatisfied: boolean;
}): boolean {
  if (isEditMode && preservesExecutionConfiguration) return true;
  return localModeSatisfied && customAiPairSatisfied;
}

function normalizeOptionalString(value: string | null | undefined): string {
  return value?.trim() ?? "";
}

function stringRecordsEqual(
  a: Readonly<Record<string, string>>,
  b: Readonly<Record<string, string>>,
): boolean {
  const aKeys = Object.keys(a);
  const bKeys = Object.keys(b);
  if (aKeys.length !== bKeys.length) return false;
  return aKeys.every((key) => key in b && a[key] === b[key]);
}
