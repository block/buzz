import { desktopFeatures, useFeatureToggle } from "@/shared/features";
import type { FeatureDefinition } from "@/shared/features";
import { Switch } from "@/shared/ui/switch";

function FeatureRow({ feature }: { feature: FeatureDefinition }) {
  const [enabled, toggle] = useFeatureToggle(feature.id);
  const switchId = `feature-toggle-${feature.id}`;

  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-background/70 px-4 py-3">
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium" id={`${switchId}-label`}>
          {feature.name}
        </p>
        <p className="text-xs text-muted-foreground">{feature.description}</p>
      </div>
      <Switch
        aria-labelledby={`${switchId}-label`}
        checked={enabled}
        data-testid={switchId}
        onCheckedChange={toggle}
      />
    </div>
  );
}

export function ExperimentalFeaturesCard() {
  const previewFeatures = desktopFeatures.filter((f) => f.tier === "preview");

  return (
    <section className="min-w-0" data-testid="settings-experimental">
      <div className="mb-12 min-w-0">
        <h2 className="text-2xl font-semibold tracking-tight">Experiments</h2>
        <p className="text-base font-normal text-muted-foreground">
          These features are functional but still being refined. Enable them to
          try new capabilities early.
        </p>
      </div>

      <div className="flex flex-col gap-2">
        {previewFeatures.map((f) => (
          <FeatureRow feature={f} key={f.id} />
        ))}
      </div>
    </section>
  );
}
