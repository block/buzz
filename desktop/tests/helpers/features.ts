// Single source of truth for E2E tests: derive the preview-feature list from
// /preview-features.json so we don't have to hand-maintain a parallel array.
//
// New preview features added to the manifest are picked up automatically by
// every test that imports from here.
import featuresManifest from "../../../preview-features.json" with {
  type: "json",
};

interface FeatureDefinition {
  id: string;
  name: string;
  description: string;
  platforms?: string[];
}

interface FeaturesManifest {
  version: number;
  features: FeatureDefinition[];
}

const manifest = featuresManifest as FeaturesManifest;

/** IDs of every preview feature on desktop. */
export const PREVIEW_FEATURE_IDS: string[] = manifest.features
  .filter((f) => !f.platforms || f.platforms.includes("desktop"))
  .map((f) => f.id);
