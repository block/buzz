import type { ExecutionNodeTarget } from "@/shared/api/tauriExecution";

import {
  executionNodeRunOnValue,
  parseExecutionNodeRunOn,
} from "./whereToRunIntent";

/** Liveness signal shown as the card's corner dot. */
export type RunOnAvailability = ExecutionNodeTarget["availability"];

/** One selectable target card in the "Run on" picker. */
export type RunOnOption = {
  /** `runOn` draft value this card selects. */
  value: string;
  kind: "local" | "execution-node" | "provider";
  label: string;
  /** Secondary line above the name ("current, unavailable"). */
  detail: string | null;
  /** Corner-dot liveness; null = no liveness signal (provider backends). */
  availability: RunOnAvailability | null;
  /** False = dimmed and not choosable (unavailable node). */
  selectable: boolean;
};

/**
 * Derive the picker's card models from discovered targets.
 *
 * Mirrors the previous `<select>` semantics exactly: "This computer" first,
 * then deploy-capable execution nodes (unavailable ones visible but not
 * selectable), then provider backends when the provider path is enabled.
 * If the current selection references a target that is no longer
 * discovered (offline node, removed provider binary), a fallback card keeps
 * it visible and selectable so the picker reflects reality instead of
 * silently dropping the selection.
 */
export function deriveRunOnOptions({
  backendProviders,
  executionNodes,
  providersEnabled,
  runOn,
}: {
  backendProviders: readonly { id: string }[];
  executionNodes: readonly Pick<
    ExecutionNodeTarget,
    "availability" | "capabilities" | "displayName" | "nodeId"
  >[];
  providersEnabled: boolean;
  runOn: string;
}): RunOnOption[] {
  const options: RunOnOption[] = [
    {
      availability: "connected",
      detail: null,
      kind: "local",
      label: "This computer",
      selectable: true,
      value: "local",
    },
  ];

  for (const node of executionNodes) {
    if (!node.capabilities.includes("deploy")) continue;
    options.push({
      availability: node.availability,
      detail: null,
      kind: "execution-node",
      label: node.displayName,
      selectable: node.availability !== "unavailable",
      value: executionNodeRunOnValue(node.nodeId),
    });
  }

  if (providersEnabled) {
    for (const provider of backendProviders) {
      options.push({
        availability: null,
        detail: null,
        kind: "provider",
        label: provider.id,
        selectable: true,
        value: provider.id,
      });
    }
  }

  if (!options.some((option) => option.value === runOn)) {
    const currentNodeId = parseExecutionNodeRunOn(runOn);
    options.push(
      currentNodeId !== null
        ? {
            availability: "unavailable",
            detail: "current, unavailable",
            kind: "execution-node",
            label: `Node ${currentNodeId.slice(0, 8)}…`,
            selectable: true,
            value: runOn,
          }
        : {
            availability: null,
            detail: "current",
            kind: "provider",
            label: runOn,
            selectable: true,
            value: runOn,
          },
    );
  }

  return options;
}
