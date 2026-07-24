/**
 * Command artefact classifications, ordered from least to most restrictive.
 *
 * `OFFICIAL` is deliberately the lowest supported classification: the command
 * domain never represents an unclassified artefact.
 */
export const CLASSIFICATIONS = [
  "OFFICIAL",
  "OFFICIAL: Sensitive",
  "PROTECTED",
  "SECRET",
  "TOP SECRET",
] as const;

export type Classification = (typeof CLASSIFICATIONS)[number];

const CLASSIFICATION_RANK = new Map<Classification, number>(
  CLASSIFICATIONS.map((classification, index) => [classification, index]),
);

export function isClassification(value: unknown): value is Classification {
  return (
    typeof value === "string" &&
    CLASSIFICATION_RANK.has(value as Classification)
  );
}

/**
 * Returns the most restrictive classification in `classifications`.
 *
 * An empty collection is still command-domain material and therefore defaults
 * to `OFFICIAL`.
 */
export function highestClassification(
  classifications: readonly Classification[],
): Classification {
  return classifications.reduce<Classification>(
    (highest, classification) =>
      (CLASSIFICATION_RANK.get(classification) ?? -1) >
      (CLASSIFICATION_RANK.get(highest) ?? -1)
        ? classification
        : highest,
    "OFFICIAL",
  );
}

/**
 * Resolves a requested classification without allowing nested artefacts to be
 * silently downgraded.
 */
export function resolveClassification(
  requested: Classification = "OFFICIAL",
  inherited: readonly Classification[] = [],
): Classification {
  return highestClassification([requested, ...inherited]);
}
