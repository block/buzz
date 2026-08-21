import { setAgentManagedProfiles } from "@/shared/api/tauri";
import { desktopFeatures, useFeatureToggle } from "@/shared/features";
import type { FeatureDefinition } from "@/shared/features";
import { Switch } from "@/shared/ui/switch";
import { ProviderUsageExperimentSettings } from "@/features/provider-usage/ui/ProviderUsageExperimentSettings";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

function FeatureRow({ feature }: { feature: FeatureDefinition }) {
  const [enabled, toggle] = useFeatureToggle(feature.id);
  const switchId = `feature-toggle-${feature.id}`;

  return (
    <SettingsOptionRow className="flex-col items-stretch gap-0">
      <div className="flex w-full items-center justify-between gap-3">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium" id={`${switchId}-label`}>
            {feature.name}
          </p>
          <p className="text-xs text-muted-foreground/70" data-settings-subcopy>
            {feature.description}
          </p>
        </div>
        <Switch
          aria-labelledby={`${switchId}-label`}
          checked={enabled}
          data-testid={switchId}
          onCheckedChange={(value) => {
            toggle(value);
            if (feature.id === "agentManagedProfiles") {
              void setAgentManagedProfiles(value).catch((error) => {
                console.error(
                  "Failed to apply agent-managed profiles setting:",
                  error,
                );
              });
            }
          }}
        />
      </div>
      {feature.id === "providerUsage" ? (
        <ProviderUsageExperimentSettings enabled={enabled} />
      ) : null}
    </SettingsOptionRow>
  );
}

export function ExperimentalFeaturesCard() {
  // Manifest is preview-only by definition; every desktop entry is a preview
  // feature.
  const previewFeatures = desktopFeatures;

  return (
    <section className="min-w-0" data-testid="settings-experimental">
      <SettingsSectionHeader
        title="Experiments"
        description={
          <>
            These features are functional but still being refined. Enable them
            to try new capabilities early.
          </>
        }
      />

      <SettingsOptionGroup title="Features">
        {previewFeatures.map((f) => (
          <FeatureRow feature={f} key={f.id} />
        ))}
      </SettingsOptionGroup>
    </section>
  );
}
