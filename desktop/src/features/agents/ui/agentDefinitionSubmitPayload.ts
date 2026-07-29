import { runtimeSupportsLlmProviderSelection } from "./agentConfigOptions";

/**
 * Pure helper extracted from the `handleSubmit` path of `AgentDefinitionDialog`
 * so the payload logic can be unit-tested without rendering the component.
 *
 * Computes the `runtime`, `model`, and `provider` fields for the definition
 * submit payload, resolving auto-seeded builtin-edit semantics: when the
 * runtime was auto-seeded (the user never explicitly chose one), it is omitted
 * from the payload, and model/provider edits are still persisted via the
 * `modelProviderEditableWithoutRuntime` path.
 */
export function buildRuntimeModelProviderPayload({
  runtime,
  model,
  provider,
  isEditMode,
  isAutoSeeded,
  initialPreviousRuntime,
  initialModel,
  initialProvider,
  initialModelProviderEditableWithoutRuntime,
  runsRemotely = false,
}: {
  runtime: string;
  model: string;
  provider: string;
  isEditMode: boolean;
  isAutoSeeded: boolean;
  initialPreviousRuntime: string;
  initialModel: string | null | undefined;
  initialProvider: string | null | undefined;
  initialModelProviderEditableWithoutRuntime: boolean;
  /**
   * "Where to run" targets a backend provider. Such a create carries no local
   * runtime by design — its harness is pinned from the HOST's catalog — but its
   * Model and LLM provider fields are still shown and still required, keyed off
   * the remote harness. Without this the blank runtime would read as "no fields
   * were visible" and silently drop the pair the user just filled in.
   */
  runsRemotely?: boolean;
}): {
  runtime: string | undefined;
  model: string | undefined;
  provider: string | undefined;
} {
  const trimmedRuntime = runtime.trim();
  const previousRuntime = initialPreviousRuntime;
  const isAutoSeededRuntimeForBuiltinEdit =
    isEditMode && previousRuntime.length === 0 && isAutoSeeded;
  const runtimeForSubmit = isAutoSeededRuntimeForBuiltinEdit
    ? ""
    : trimmedRuntime;
  // An auto-seeded builtin edit is treated the same as an existing builtin with
  // a saved model/provider: the field is editable without a runtime, and the
  // user's model/provider choice is persisted in the payload.
  const modelProviderEditableWithoutRuntime =
    (initialModelProviderEditableWithoutRuntime ||
      isAutoSeededRuntimeForBuiltinEdit ||
      runsRemotely) &&
    runtimeForSubmit.length === 0;
  const llmProviderVisibleForSubmit =
    (runtimeForSubmit.length > 0 &&
      runtimeSupportsLlmProviderSelection(runtimeForSubmit)) ||
    modelProviderEditableWithoutRuntime;
  const shouldPreserveHiddenModelProvider =
    isEditMode &&
    previousRuntime.length === 0 &&
    runtimeForSubmit.length === 0 &&
    !modelProviderEditableWithoutRuntime;
  return {
    runtime: runtimeForSubmit || undefined,
    model:
      runtimeForSubmit || modelProviderEditableWithoutRuntime
        ? model.trim() || undefined
        : shouldPreserveHiddenModelProvider
          ? (initialModel ?? undefined)
          : undefined,
    provider: llmProviderVisibleForSubmit
      ? provider.trim() || undefined
      : shouldPreserveHiddenModelProvider
        ? (initialProvider ?? undefined)
        : undefined,
  };
}
