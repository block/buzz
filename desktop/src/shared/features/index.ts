export { FeatureGate } from "./FeatureGate";
export { allFeatures, desktopFeatures, getFeature } from "./manifest";
export { getOverrides, setOverride, clearOverride } from "./store";
export type {
  FeatureDefinition,
  FeaturesManifest,
  FeaturePlatform,
  FeatureTier,
} from "./types";
export {
  useFeatureEnabled,
  useFeatureToggle,
  useFeatureSnapshot,
  resolveEnabled,
} from "./useFeatureEnabled";
