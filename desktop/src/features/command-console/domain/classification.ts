/**
 * The complete persisted classification vocabulary for Command Console data.
 */
export const CLASSIFICATIONS = ["PUBLIC", "OFFICIAL"] as const;

export type Classification = (typeof CLASSIFICATIONS)[number];

export function isClassification(value: unknown): value is Classification {
  return value === "PUBLIC" || value === "OFFICIAL";
}

/**
 * Defaults new artefacts to OFFICIAL and elevates a PUBLIC composite whenever
 * any nested artefact is OFFICIAL.
 */
export function resolveClassification(
  requested: Classification = "OFFICIAL",
  inherited: readonly Classification[] = [],
): Classification {
  return requested === "OFFICIAL" || inherited.includes("OFFICIAL")
    ? "OFFICIAL"
    : "PUBLIC";
}
