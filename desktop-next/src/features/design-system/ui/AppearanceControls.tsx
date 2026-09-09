import { Button } from "@buzz/ui";
import { useColorScheme } from "@/shared/theme/useColorScheme";
import { useDensity } from "@/shared/theme/DensityProvider";

/** Independent appearance preferences for every design-system page. */
export function AppearanceControls() {
  const { scheme, toggle } = useColorScheme();
  const { density, setDensity, saveError } = useDensity();

  return (
    <div className="mx-3 flex shrink-0 flex-col items-start gap-3">
      <Button
        size="sm"
        variant="secondary"
        onClick={toggle}
        aria-label={`Switch to ${scheme === "light" ? "dark" : "light"} mode`}
      >
        {scheme === "light" ? "Dark mode" : "Light mode"}
      </Button>
      <fieldset className="min-w-0">
        <legend className="mb-2 text-meta text-tertiary">Density</legend>
        <div className="flex flex-wrap gap-1">
          {(["normal", "compact"] as const).map((value) => (
            <Button
              key={value}
              size="sm"
              variant={density === value ? "primary" : "ghost"}
              aria-pressed={density === value}
              onClick={() => setDensity(value)}
            >
              {value === "normal" ? "Normal" : "Compact"}
            </Button>
          ))}
        </div>
      </fieldset>
      {saveError && (
        <p role="status" className="text-caption text-secondary">
          {saveError}
        </p>
      )}
    </div>
  );
}
