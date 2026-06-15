import * as React from "react";
import { AlertCircle, Network } from "lucide-react";

import { meshAgentPreset, type MeshServeTarget } from "@/shared/api/tauriMesh";
import { Switch } from "@/shared/ui/switch";

import {
  detectMeshPresetOverrides,
  meshAgentPresetPatch,
} from "../applyMeshAgentPreset";
import { useMeshAvailability } from "../hooks/useMeshAvailability";

/**
 * The "Run on relay mesh" entry inside CreateAgentDialog. When enabled, the
 * user picks a model from `mesh_availability().models`, the dialog's runtime
 * + env-var state is fanned out from `mesh_agent_preset()`, and the existing
 * runtime/backend pickers are hidden in the parent.
 *
 * Lives outside `CreateAgentDialog` so the dialog file's diff stays narrow —
 * the dialog only renders this and reacts to its `onPatch` and `useMesh`
 * callbacks. The dialog is the single source of truth for state; this
 * component is purely a controller for the mesh-specific subset.
 */
export function RelayMeshAgentSection({
  current,
  useMesh,
  targetEndpointAddr,
  onUseMeshChange,
  onModelIdChange,
  onTargetChange,
}: {
  /**
   * Current draft state of the *fields the preset would overwrite*. Used to
   * compute the override warning ("Using Relay mesh — overrides this
   * persona's model") and to know what changed.
   */
  current: {
    acpCommand: string;
    agentCommand: string;
    agentArgs: string[];
    mcpCommand: string;
    model: string | null;
    envVars: Record<string, string>;
  };
  useMesh: boolean;
  modelId: string; // Parent-owned selected model id; retained for API symmetry with onModelIdChange.
  targetEndpointAddr: string;
  onUseMeshChange: (next: boolean) => void;
  /**
   * Fires when the user picks a model. The parent should fan out the
   * preset to its individual setters (acpCommand, envVars, etc.) using
   * `meshAgentPresetPatch`. We expose both `modelId` and the resolved patch
   * so the parent can choose either eager fan-out or lazy-apply on submit.
   */
  onModelIdChange: (
    nextModelId: string,
    patch: ReturnType<typeof meshAgentPresetPatch> | null,
  ) => void;
  onTargetChange: (target: MeshServeTarget | null) => void;
}) {
  const { availability, error } = useMeshAvailability();
  const [presetError, setPresetError] = React.useState<string | null>(null);

  const disabled = availability == null || !availability.available;
  const disabledReason =
    availability == null
      ? (error ?? "Checking relay mesh availability…")
      : (availability.reason ?? "Relay compute isn't available right now.");

  // Compute overrides from the currently-selected model's preset, *not* from
  // an arbitrary one — the warning must reflect what'll actually happen.
  const [overrides, setOverrides] = React.useState<string[]>([]);

  const targets = availability?.serveTargets ?? [];
  const selectedValue = targetEndpointAddr;

  async function pickTarget(endpointAddr: string) {
    if (endpointAddr === "") {
      onTargetChange(null);
      onModelIdChange("", null);
      setOverrides([]);
      setPresetError(null);
      return;
    }
    const target = targets.find(
      (candidate) => candidate.endpointAddr === endpointAddr,
    );
    if (!target) {
      onTargetChange(null);
      onModelIdChange("", null);
      setPresetError("Selected relay mesh target is no longer available.");
      return;
    }
    setPresetError(null);
    try {
      const preset = await meshAgentPreset(target.modelId);
      const patch = meshAgentPresetPatch(preset);
      setOverrides(detectMeshPresetOverrides(current, preset));
      onTargetChange(target);
      onModelIdChange(target.modelId, patch);
    } catch (err) {
      setPresetError(err instanceof Error ? err.message : String(err));
      onTargetChange(target);
      onModelIdChange(target.modelId, null);
    }
  }

  function targetLabel(target: MeshServeTarget) {
    const model = target.modelName ?? target.modelId;
    const device = target.deviceName ?? target.nodeName ?? target.endpointId;
    return device ? `${model} — ${device}` : model;
  }

  return (
    <div className="space-y-3 rounded-2xl border border-border/70 bg-muted/10 px-4 py-3">
      <div className="flex items-center justify-between gap-4">
        <div className="min-w-0">
          <label
            className="flex items-center gap-1.5 text-sm font-medium"
            htmlFor="agent-relay-mesh-toggle"
          >
            <Network className="h-4 w-4 text-muted-foreground" />
            Run on relay mesh
          </label>
          <p className="text-sm text-muted-foreground">
            {disabled
              ? disabledReason
              : "Use a member's shared compute — no API key needed."}
          </p>
        </div>
        <Switch
          checked={useMesh}
          data-testid="agent-relay-mesh-toggle"
          disabled={disabled}
          id="agent-relay-mesh-toggle"
          onCheckedChange={onUseMeshChange}
        />
      </div>

      {useMesh ? (
        <div className="space-y-2">
          <label
            className="text-xs text-muted-foreground"
            htmlFor="agent-relay-mesh-model"
          >
            Model
          </label>
          <select
            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
            data-testid="agent-relay-mesh-model"
            id="agent-relay-mesh-model"
            onChange={(e) => void pickTarget(e.target.value)}
            value={selectedValue}
          >
            <option value="">Choose a target…</option>
            {targets.map((target) => (
              <option key={target.endpointAddr} value={target.endpointAddr}>
                {targetLabel(target)}
              </option>
            ))}
          </select>
          {presetError ? (
            <p className="rounded bg-destructive/10 px-2 py-1 text-xs text-destructive">
              Couldn't load model preset: {presetError}
            </p>
          ) : null}
          {overrides.length > 0 ? (
            <p className="flex items-start gap-1.5 rounded bg-warning-bg/60 px-2 py-1.5 text-xs text-warning">
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
              <span>
                Using Relay mesh overrides this agent's {overrides.join(", ")}.
              </span>
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
