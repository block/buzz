import * as React from "react";

import type {
  AcpRuntimeCatalogEntry,
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import type { AgentAiConfigurationMode } from "./AgentAiConfigurationMode";
import {
  createRuntimeSeedAction,
  createRuntimeSeedAllowed,
} from "./createRuntimeGate";

type CreateRuntimeSeedInput = {
  aiConfigurationMode: AgentAiConfigurationMode;
  /** "Where to run" targets a backend provider — see `createRuntimeSeedAllowed`. */
  createRunsRemotely: boolean;
  /** This edit's record is provider-backed — see `createRuntimeSeedAction`. */
  editsProviderRecord: boolean;
  defaultRuntime: AcpRuntimeCatalogEntry | null;
  initialValues: CreatePersonaInput | UpdatePersonaInput | null;
  open: boolean;
  runtime: string;
  runtimesLoading: boolean;
  setRuntime: (next: string) => void;
  /**
   * Owned by the dialog rather than this hook: the dropdown clears the
   * auto-seeded flag on an explicit pick, submit reads it to decide whether to
   * omit the runtime, and the open-reset effect clears both.
   */
  isRuntimeAutoSeededRef: React.MutableRefObject<boolean>;
  hasSeededForOpenRef: React.MutableRefObject<boolean>;
};

/**
 * Seed the definition dialog's harness field from this computer's default, and
 * shed that seed when the create turns out to run somewhere else.
 *
 * Extracted from `AgentDefinitionDialog` so the seed's whole lifecycle — when
 * it applies, when it is dropped, and what it must never overwrite — lives in
 * one place and is unit-testable without rendering the dialog.
 */
export function useCreateRuntimeSeed({
  aiConfigurationMode,
  createRunsRemotely,
  editsProviderRecord,
  defaultRuntime,
  initialValues,
  open,
  runtime,
  runtimesLoading,
  setRuntime,
  isRuntimeAutoSeededRef,
  hasSeededForOpenRef,
}: CreateRuntimeSeedInput) {
  React.useEffect(() => {
    const action = createRuntimeSeedAction({
      defaultRuntimeId: defaultRuntime?.id ?? null,
      definitionRuntime: initialValues?.runtime,
      editsProviderRecord,
      hasInitialValues: initialValues !== null,
      hasSeededForOpen: hasSeededForOpenRef.current,
      isAutoSeeded: isRuntimeAutoSeededRef.current,
      open,
      runsRemotely: createRunsRemotely,
      runtime,
      runtimesLoading,
    });
    if (action.type === "none") return;
    if (action.type === "shed") {
      setRuntime("");
      isRuntimeAutoSeededRef.current = false;
      hasSeededForOpenRef.current = false;
      return;
    }

    setRuntime(action.runtimeId);
    hasSeededForOpenRef.current = true;
    // Marked in BOTH modes: the flag means "this value came from the local
    // default, not the user", which is exactly what the shed above must be able
    // to recognise on a create that later turns remote. It only reaches the
    // submit payload in edit mode (`buildRuntimeModelProviderPayload` reads it
    // under `isEditMode`), where it omits an auto-seeded runtime for builtin
    // definitions whose canonical runtime is null. Explicit user changes via
    // the dropdown clear it.
    isRuntimeAutoSeededRef.current = true;
  }, [
    createRunsRemotely,
    defaultRuntime,
    editsProviderRecord,
    hasSeededForOpenRef,
    initialValues,
    isRuntimeAutoSeededRef,
    open,
    runtime,
    runtimesLoading,
    setRuntime,
  ]);

  // Keep an inherited Create runtime synced with defaults saved in-place.
  // Create-only (`"id" in initialValues` bails on every edit), so the
  // provider-record guard has nothing to add here.
  React.useEffect(() => {
    if (
      !open ||
      !initialValues ||
      "id" in initialValues ||
      initialValues.runtime?.trim() ||
      aiConfigurationMode !== "defaults" ||
      runtimesLoading ||
      defaultRuntime === null ||
      !createRuntimeSeedAllowed(createRunsRemotely) ||
      (runtime.trim().length > 0 && !isRuntimeAutoSeededRef.current)
    ) {
      return;
    }

    if (runtime !== defaultRuntime.id) setRuntime(defaultRuntime.id);
    isRuntimeAutoSeededRef.current = true;
    hasSeededForOpenRef.current = true;
  }, [
    aiConfigurationMode,
    createRunsRemotely,
    defaultRuntime,
    hasSeededForOpenRef,
    initialValues,
    isRuntimeAutoSeededRef,
    open,
    runtime,
    runtimesLoading,
    setRuntime,
  ]);
}
