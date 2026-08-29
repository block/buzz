import * as React from "react";

import {
  computeLocalModeGate,
  shouldClearKnownModelForSelectionScope,
} from "./agentConfigOptions";

type LocalModeGateOptions = Parameters<typeof computeLocalModeGate>[0];

/**
 * Memoized local-mode credential gate for the agent definition dialog.
 * Extracted so the dialog stays under the file-size ceiling; semantics are
 * identical to the previous inline useMemo.
 */
export function useLocalModeGate(options: LocalModeGateOptions) {
  const {
    bakedEnvKeys,
    envVars,
    globalEnvVars,
    globalModel,
    globalProvider,
    isProviderMode,
    model,
    provider,
    providerEnvVar,
    setupFields,
    runtimeFileConfig,
    runtimeId,
  } = options;
  return React.useMemo(
    () =>
      computeLocalModeGate({
        bakedEnvKeys,
        envVars,
        globalEnvVars,
        globalModel,
        globalProvider,
        isProviderMode,
        model,
        provider,
        providerEnvVar,
        setupFields,
        runtimeFileConfig,
        runtimeId,
      }),
    [
      bakedEnvKeys,
      envVars,
      globalEnvVars,
      globalModel,
      globalProvider,
      isProviderMode,
      model,
      provider,
      providerEnvVar,
      setupFields,
      runtimeFileConfig,
      runtimeId,
    ],
  );
}

type ResetModelOnRuntimeChangeOptions = {
  open: boolean;
  modelFieldVisible: boolean;
  isCustomModelEditing: boolean;
  model: string;
  effectiveProvider: string;
  providerEnvVar: string | null | undefined;
  runtime: string;
  setModel: (value: string) => void;
  setIsCustomModelEditing: (value: boolean) => void;
};

/**
 * Clears a known-model selection when the runtime/provider scope changes
 * underneath it. Mirrors the previous inline effect in the definition dialog.
 */
export function useResetModelOnRuntimeChange({
  open,
  modelFieldVisible,
  isCustomModelEditing,
  model,
  effectiveProvider,
  providerEnvVar,
  runtime,
  setModel,
  setIsCustomModelEditing,
}: ResetModelOnRuntimeChangeOptions) {
  React.useEffect(() => {
    const scopeChanged = shouldClearKnownModelForSelectionScope({
      model,
      provider: effectiveProvider,
      providerEnvVar,
      runtime,
    });
    if (
      open === false ||
      modelFieldVisible === false ||
      isCustomModelEditing ||
      scopeChanged === false
    ) {
      return;
    }

    setModel("");
    setIsCustomModelEditing(false);
  }, [
    effectiveProvider,
    isCustomModelEditing,
    model,
    modelFieldVisible,
    open,
    providerEnvVar,
    runtime,
    setIsCustomModelEditing,
    setModel,
  ]);
}
