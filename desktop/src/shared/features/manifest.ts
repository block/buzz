import manifestJson from "@features-manifest";
import { z } from "zod";
import type { FeatureDefinition, FeaturesManifest } from "./types";

// Schema — runtime-validates the bundled preview-features.json at startup.
//
// On parse failure we fall back to an empty manifest and log a console warning.
// The app keeps working; gated UI stays hidden; nothing accidentally leaks.

const FeaturePlatformSchema = z.enum(["desktop", "mobile"]);

const FeatureDefinitionSchema = z.object({
  id: z.string().min(1),
  name: z.string().min(1),
  description: z.string(),
  platforms: z.array(FeaturePlatformSchema).optional(),
});

const FeaturesManifestSchema = z.object({
  version: z.number().int().nonnegative(),
  features: z.array(FeatureDefinitionSchema),
});

const EMPTY_MANIFEST: FeaturesManifest = { version: 1, features: [] };

function loadManifest(): FeaturesManifest {
  const result = FeaturesManifestSchema.safeParse(manifestJson);
  if (!result.success) {
    console.warn(
      "[FeatureFlags] preview-features.json failed schema validation; falling back to empty manifest.",
      result.error.issues,
    );
    return EMPTY_MANIFEST;
  }
  return result.data;
}

const manifest = loadManifest();

/** The validated manifest. Use `manifest.version` for cache/storage keys. */
export { manifest };

/** All features defined in the manifest */
export const allFeatures: FeatureDefinition[] = manifest.features;

/** Only features available on desktop */
export const desktopFeatures: FeatureDefinition[] = manifest.features.filter(
  (f) => !f.platforms || f.platforms.includes("desktop"),
);

/** Look up a feature by id */
export function getFeature(id: string): FeatureDefinition | undefined {
  return manifest.features.find((f) => f.id === id);
}
