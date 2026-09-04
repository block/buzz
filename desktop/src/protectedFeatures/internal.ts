import type { FeatureDefinition } from "@/shared/features/types";

/** Definitions available only in the protected internal application build. */
export const protectedFeatureDefinitions: FeatureDefinition[] = [
  {
    id: "bestie",
    name: "Bestie",
    description: "Try a personal agent that is always close at hand",
    platforms: ["desktop"],
  },
];
