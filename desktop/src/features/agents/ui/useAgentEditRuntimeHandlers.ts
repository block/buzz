/**
 * useAgentEditRuntimeHandlers — runtime/model/provider selection handlers for
 * AgentEditMergedDialog.
 *
 * Extracted to keep the dialog under the 1000-line file-size gate. Owns the
 * D/I selection objects, apply helpers, and the four handler callbacks.
 */

import type * as React from "react";

import {
  runtimeDropdownAction,
  usePendingHarnessSelection,
} from "./addCustomHarness";
import {
  AUTO_PROVIDER_DROPDOWN_VALUE,
  runtimeSupportsLlmProviderSelection,
} from "./agentConfigOptions";
import {
  selectionOnModelDropdownChange,
  selectionOnProviderDropdownChange,
  selectionOnRuntimeChange,
  type RuntimeModelProviderSelection,
} from "./runtimeModelProviderSelection";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

export type RuntimeHandlersInput = {
  showDef: boolean;
  showInst: boolean;
  // D-state
  dProvider: string;
  dModel: string;
  dIsCustomProviderEditing: boolean;
  dIsCustomModelEditing: boolean;
  setDProvider: (v: string) => void;
  setDModel: (v: string) => void;
  setDIsCustomProviderEditing: (v: boolean) => void;
  setDIsCustomModelEditing: (v: boolean) => void;
  envVars: Record<string, string>;
  setEnvVars: (v: Record<string, string>) => void;
  definitionRuntimeId: string;
  setDefinitionRuntimeId: (v: string) => void;
  // I-state
  iProvider: string;
  iModel: string;
  iIsCustomProviderEditing: boolean;
  iIsCustomModelEditing: boolean;
  setIProvider: (v: string) => void;
  setIModel: (v: string) => void;
  setIIsCustomProviderEditing: (v: boolean) => void;
  setIIsCustomModelEditing: (v: boolean) => void;
  instanceEnvVars: Record<string, string>;
  setInstanceEnvVars: (v: Record<string, string>) => void;
  selectedRuntimeId: string;
  setSelectedRuntimeId: (v: string) => void;
  setInheritHarness: (v: boolean) => void;
  setAgentCommand: (v: string) => void;
  setAgentArgs: (v: string) => void;
  runtimeTouched: React.RefObject<boolean>;
  setIsAddHarnessOpen: (v: boolean) => void;
  runtimes: readonly AcpRuntimeCatalogEntry[];
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
  open: boolean;
};

/**
 * Returns the four runtime/model/provider change handlers and selectSavedHarness.
 *
 * D-owned handlers write to D state only (via applyDSelection).
 * I-owned handlers write to I state only (via applyISelection).
 * In definition-only context (showInst=false), D and I state are mirrored.
 */
export function useAgentEditRuntimeHandlers(input: RuntimeHandlersInput) {
  const {
    showDef,
    showInst,
    dProvider,
    dModel,
    dIsCustomProviderEditing,
    dIsCustomModelEditing,
    setDProvider,
    setDModel,
    setDIsCustomProviderEditing,
    setDIsCustomModelEditing,
    envVars,
    setEnvVars,
    definitionRuntimeId,
    setDefinitionRuntimeId,
    iProvider,
    iModel,
    iIsCustomProviderEditing,
    iIsCustomModelEditing,
    setIProvider,
    setIModel,
    setIIsCustomProviderEditing,
    setIIsCustomModelEditing,
    instanceEnvVars,
    setInstanceEnvVars,
    selectedRuntimeId,
    setSelectedRuntimeId,
    setInheritHarness,
    setAgentCommand,
    setAgentArgs,
    runtimeTouched,
    setIsAddHarnessOpen,
    runtimes,
    selectedRuntime,
    open,
  } = input;

  const dSelection: RuntimeModelProviderSelection = {
    provider: dProvider,
    model: dModel,
    isCustomProviderEditing: dIsCustomProviderEditing,
    isCustomModelEditing: dIsCustomModelEditing,
    envVars,
  };

  const iSelection: RuntimeModelProviderSelection = {
    provider: iProvider,
    model: iModel,
    isCustomProviderEditing: iIsCustomProviderEditing,
    isCustomModelEditing: iIsCustomModelEditing,
    envVars: instanceEnvVars,
  };

  /** Writes D model/provider/env. In D-only context, mirrors to I as well. */
  function applyDSelection(next: RuntimeModelProviderSelection) {
    setDProvider(next.provider);
    setDModel(next.model);
    setDIsCustomProviderEditing(next.isCustomProviderEditing);
    setDIsCustomModelEditing(next.isCustomModelEditing);
    setEnvVars(next.envVars);
    if (!showInst) {
      setIProvider(next.provider);
      setIModel(next.model);
      setIIsCustomProviderEditing(next.isCustomProviderEditing);
      setIIsCustomModelEditing(next.isCustomModelEditing);
    }
  }

  /** Writes I model/provider/env only. Never touches D state. */
  function applyISelection(next: RuntimeModelProviderSelection) {
    setIProvider(next.provider);
    setIModel(next.model);
    setIIsCustomProviderEditing(next.isCustomProviderEditing);
    setIIsCustomModelEditing(next.isCustomModelEditing);
    setInstanceEnvVars(next.envVars);
  }

  function handleDefinitionRuntimeChange(nextValue: string) {
    const action = runtimeDropdownAction(nextValue);
    if (action.kind === "add-custom-harness") {
      setIsAddHarnessOpen(true);
      return;
    }
    const nextRuntimeId = action.runtimeId;
    const previousRuntimeId = definitionRuntimeId;
    setDefinitionRuntimeId(nextRuntimeId || "custom");
    applyDSelection(
      selectionOnRuntimeChange(dSelection, {
        previousRuntime: previousRuntimeId,
        nextRuntime: nextRuntimeId,
        nextRuntimeCanChooseProvider:
          runtimeSupportsLlmProviderSelection(nextRuntimeId),
        lockedRuntimeReset: "full",
      }),
    );
  }

  function handleRuntimeDropdownChange(nextValue: string) {
    const action = runtimeDropdownAction(nextValue);
    if (action.kind === "add-custom-harness") {
      setIsAddHarnessOpen(true);
      return;
    }
    const nextRuntimeId = action.runtimeId;
    const previousRuntimeId = selectedRuntimeId;
    runtimeTouched.current = true;
    const nextRuntime = runtimes.find((r) => r.id === nextRuntimeId);
    const resolvedRuntimeId = nextRuntimeId || "custom";
    setSelectedRuntimeId(resolvedRuntimeId);
    if (resolvedRuntimeId === "custom" || nextRuntime?.command) {
      setInheritHarness(false);
    }
    if (nextRuntime?.command) {
      setAgentCommand(nextRuntime.command);
      setAgentArgs(nextRuntime.defaultArgs.join(","));
    }
    applyISelection(
      selectionOnRuntimeChange(iSelection, {
        previousRuntime: previousRuntimeId,
        nextRuntime: nextRuntimeId,
        nextRuntimeCanChooseProvider:
          runtimeSupportsLlmProviderSelection(nextRuntimeId),
        lockedRuntimeReset: "full",
      }),
    );
  }

  const selectSavedHarness = usePendingHarnessSelection(
    runtimes,
    showDef && !showInst
      ? handleDefinitionRuntimeChange
      : handleRuntimeDropdownChange,
    open,
  );

  function handleProviderDropdownChange(nextValue: string) {
    const nextProvider =
      nextValue === AUTO_PROVIDER_DROPDOWN_VALUE ? "" : nextValue;
    if (nextProvider === "relay-mesh" && selectedRuntimeId !== "buzz-agent") {
      handleRuntimeDropdownChange("buzz-agent");
    }
    const sel = showDef ? dSelection : iSelection;
    const applyFn = showDef ? applyDSelection : applyISelection;
    const nextSelection = selectionOnProviderDropdownChange(sel, {
      runtime:
        nextProvider === "relay-mesh"
          ? "buzz-agent"
          : (selectedRuntime?.id ?? selectedRuntimeId),
      nextValue,
      clearModelWhenApiKeyMissing: !showInst,
    });
    applyFn({
      ...nextSelection,
      model: nextProvider === "relay-mesh" ? "auto" : nextSelection.model,
    });
  }

  function handleModelDropdownChange(nextValue: string) {
    const sel = showDef ? dSelection : iSelection;
    const applyFn = showDef ? applyDSelection : applyISelection;
    applyFn(
      selectionOnModelDropdownChange(sel, {
        nextValue,
        clearKnownModelOnCustomEntry: !showInst,
        isModelCustom: false,
      }),
    );
  }

  return {
    handleDefinitionRuntimeChange,
    handleRuntimeDropdownChange,
    handleProviderDropdownChange,
    handleModelDropdownChange,
    selectSavedHarness,
  };
}
