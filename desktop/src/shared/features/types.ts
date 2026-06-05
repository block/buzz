/** Feature visibility tiers */
export type FeatureTier = "stable" | "preview" | "unstable";

/** Platforms a feature is available on */
export type FeaturePlatform = "desktop" | "mobile";

/** A single feature definition from the manifest */
export interface FeatureDefinition {
  id: string;
  name: string;
  description: string;
  tier: FeatureTier;
  /** If omitted, feature is available on all platforms */
  platforms?: FeaturePlatform[];
}

/** The root manifest schema */
export interface FeaturesManifest {
  version: number;
  features: FeatureDefinition[];
}
