import type {
  MeshExperimentalCatalog,
  MeshSplitEntry,
} from "@/shared/api/tauriMesh";

/**
 * Pure projection for the experimental (split) tier of Advanced settings.
 *
 * ## Why "beyond this machine" is the partition
 *
 * Splittability alone is not the interesting fact. A model can carry a layer
 * package and still fit on one machine — `Qwen3-8B-Q4_K_M` is 5 GB and
 * split-capable — and mesh-llm agrees about precedence:
 * `evaluate_model_target_capacity` returns `SingleNodeFit` first and only falls
 * back to `SplitCandidate` when no single node can hold the model.
 *
 * So offering every split-capable model here would tell someone a 5 GB model
 * needs a cluster. The experimental tier is the intersection: split-capable
 * **and** beyond local memory. Anything that fits belongs in the ordinary
 * picker, where it starts immediately.
 *
 * ## Why the consequence is stated before selection, not after
 *
 * A collective model does not load on demand. mesh-llm gathers a cohort over
 * gossip and holds a settle barrier — at least two participants, membership
 * unchanged for a dwell period — then elects a coordinator deterministically by
 * VRAM. Until that forms, a node named for a collective model sits waiting.
 *
 * Nothing about that looks different from a slow download, so a person who was
 * not told would reasonably read it as a hang and file a bug. The disclosure is
 * therefore part of the option, not a footnote after the fact.
 */

export type MeshExperimentalOption = {
  entry: MeshSplitEntry;
  /** e.g. "134 GB · 80 layers", or "61 layers" when no byte size is published. */
  detail: string;
};

export type MeshExperimentalModel = {
  /** Split-capable models that exceed this machine — the experimental offer. */
  options: MeshExperimentalOption[];
  /**
   * Why the list is empty, or null when it has entries.
   *
   * An empty list has three quite different causes and they must not read
   * alike: the catalog could not be read, it holds no packages at all, or this
   * machine is simply large enough that nothing needs splitting. The third is
   * good news and must not look like a failure.
   */
  emptyReason:
    | "loading"
    | "catalogUnavailable"
    | "noPackages"
    | "machineFitsEverything"
    | null;
};

/** Bytes → GB label, matching the rest of the mesh surfaces. */
function formatGb(gb: number): string {
  return `${gb >= 10 ? Math.round(gb) : Math.round(gb * 10) / 10} GB`;
}

/**
 * One line describing reach: size where known, layers always.
 *
 * Layers are the unit actually distributed, so they are the honest figure when
 * the catalog publishes a layer count in place of bytes (two upstream entries
 * do). Never synthesizes a size from a layer count — they are not convertible.
 */
export function describeSplitEntry(entry: MeshSplitEntry): string {
  const parts: string[] = [];
  if (entry.sizeGb !== null) {
    parts.push(formatGb(entry.sizeGb));
  }
  if (entry.layerCount !== null) {
    parts.push(`${entry.layerCount} layers`);
  }
  if (parts.length === 0) {
    // Neither figure published. Say nothing rather than invent a number.
    return entry.sizeLabel ?? "";
  }
  return parts.join(" · ");
}

export function deriveMeshExperimentalModel(
  catalog: MeshExperimentalCatalog | null | undefined,
): MeshExperimentalModel {
  if (catalog === undefined) {
    return { options: [], emptyReason: "loading" };
  }
  if (catalog === null) {
    return { options: [], emptyReason: "catalogUnavailable" };
  }
  if (catalog.entries.length === 0) {
    return {
      options: [],
      emptyReason: catalog.stale ? "catalogUnavailable" : "noPackages",
    };
  }
  const beyondThisMachine = catalog.entries.filter(
    (entry) => !entry.fitsLocally,
  );
  if (beyondThisMachine.length === 0) {
    // Every split-capable model fits here. Not a failure — the machine is big
    // enough that splitting buys nothing.
    return { options: [], emptyReason: "machineFitsEverything" };
  }
  return {
    options: beyondThisMachine.map((entry) => ({
      entry,
      detail: describeSplitEntry(entry),
    })),
    emptyReason: null,
  };
}

/** Copy for an empty experimental tier, keyed on why. */
export function describeExperimentalEmpty(
  reason: NonNullable<MeshExperimentalModel["emptyReason"]>,
): string {
  switch (reason) {
    case "loading":
      return "Checking for models that can run across machines…";
    case "catalogUnavailable":
      return "Can't reach the model catalog right now.";
    case "noPackages":
      return "No models in the catalog can run across machines yet.";
    case "machineFitsEverything":
      // Deliberately not phrased as an absence: this is the good outcome.
      return "This computer can run every model in the catalog on its own.";
  }
}
