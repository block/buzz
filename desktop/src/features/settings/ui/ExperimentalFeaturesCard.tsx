import { desktopFeatures, useFeatureToggle } from "@/shared/features";
import type { FeatureDefinition } from "@/shared/features";
import { Switch } from "@/shared/ui/switch";

function FeatureRow({ feature }: { feature: FeatureDefinition }) {
  const [enabled, toggle] = useFeatureToggle(feature.id);

  return (
    <label className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-background/70 px-4 py-3">
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">{feature.name}</p>
        <p className="text-xs text-muted-foreground">{feature.description}</p>
      </div>
      <Switch
        checked={enabled}
        data-testid={`feature-toggle-${feature.id}`}
        onCheckedChange={toggle}
      />
    </label>
  );
}

export function ExperimentalFeaturesCard() {
  const previewFeatures = desktopFeatures.filter(
    (f) => f.tier === "preview",
  );
  const unstableFeatures = desktopFeatures.filter(
    (f) => f.tier === "unstable",
  );

  return (
    <section className="min-w-0" data-testid="settings-experimental">
      <div className="mb-3 min-w-0">
        <h2 className="text-sm font-semibold tracking-tight">
          Preview
        </h2>
        <p className="text-sm text-muted-foreground">
          These features are functional but still being refined. Enable them to
          try new capabilities early.
        </p>
      </div>

      <div className="flex flex-col gap-2">
        {previewFeatures.map((f) => (
          <FeatureRow feature={f} key={f.id} />
        ))}
      </div>

      {unstableFeatures.length > 0 && (
        <>
          <div className="mb-3 mt-6 min-w-0">
            <h2 className="text-sm font-semibold tracking-tight">
              Unstable
            </h2>
            <p className="text-sm text-muted-foreground">
              Here be dragons — these features might break or disappear. Enable
              at your own risk.
            </p>
          </div>

          <div className="flex flex-col gap-2">
            {unstableFeatures.map((f) => (
              <FeatureRow feature={f} key={f.id} />
            ))}
          </div>
        </>
      )}
    </section>
  );
}
