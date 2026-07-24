import type { MeshAvailability } from "@/shared/api/tauriMesh";

/**
 * Presentation-ready description of where a relay-mesh agent's inference
 * actually runs: which model, and how many live serving nodes on this relay
 * currently host it.
 */
export type MeshInferenceLocation = {
  /** Compact label for inline display (header/trace chrome). */
  label: string;
  /** Full sentence for the tooltip. */
  title: string;
  /** Distinct live serving nodes for the agent's model (0 = none live). */
  nodeCount: number;
};

/**
 * Mirror of the Rust-side canonical model matching in
 * `pick_serve_target_for_model` (commands/mesh_llm.rs): trim and drop the
 * implicit `@main` revision so `org/model@main:q4` matches `org/model:q4`.
 */
function canonicalModelId(value: string): string {
  return value.trim().replace("@main", "");
}

function distinctNodeCount(targets: MeshAvailability["serveTargets"]): number {
  const nodes = new Set<string>();
  for (const target of targets) {
    nodes.add(target.deviceId ?? target.endpointId ?? target.endpointAddr);
  }
  return nodes.size;
}

/**
 * Derive the inference-location description for a relay-mesh agent.
 *
 * `model` is the agent record's model ref (`"auto"`/empty = mesh router
 * picks per request). Returns `null` while availability is unknown
 * (loading/error) — the indicator renders nothing rather than guessing.
 */
export function describeMeshInferenceLocation({
  availability,
  model,
}: {
  availability: MeshAvailability | null;
  model: string | null;
}): MeshInferenceLocation | null {
  if (availability === null) {
    return null;
  }

  const trimmedModel = (model ?? "").trim();
  const isAuto = trimmedModel === "" || trimmedModel === "auto";

  const targets = isAuto
    ? availability.serveTargets
    : availability.serveTargets.filter(
        (target) =>
          canonicalModelId(target.modelId) === canonicalModelId(trimmedModel),
      );
  const nodeCount = distinctNodeCount(targets);

  if (nodeCount === 0) {
    return {
      label: "Shared compute · no live serving nodes",
      title:
        "This agent runs inference on Buzz shared compute, but no member is currently serving its model on this relay.",
      nodeCount,
    };
  }

  const nodesPhrase = `${nodeCount} node${nodeCount === 1 ? "" : "s"}`;
  if (isAuto) {
    return {
      label: `Shared compute · auto-routed · ${nodesPhrase}`,
      title: `This agent runs inference on Buzz shared compute, auto-routed across ${nodesPhrase} serving on this relay.`,
      nodeCount,
    };
  }

  const displayName =
    targets.find((target) => target.modelName?.trim())?.modelName?.trim() ??
    trimmedModel;
  return {
    label: `Shared compute · ${displayName} · ${nodesPhrase}`,
    title: `This agent runs inference with ${displayName} on ${nodesPhrase} serving on this relay via Buzz shared compute.`,
    nodeCount,
  };
}
