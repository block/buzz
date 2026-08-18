import { Cloud, Laptop, Unplug } from "lucide-react";

import type { ModelRoutingPreference } from "@/shared/api/tauriCommandBrief";
import { Button } from "@/shared/ui/button";

export function ModelRoutingControls({
  preference,
  activeWork,
  disabled,
  error,
  onChange,
}: {
  preference: ModelRoutingPreference;
  activeWork: boolean;
  disabled: boolean;
  error: string | null;
  onChange: (preference: ModelRoutingPreference) => void;
}) {
  const controlsDisabled = disabled || activeWork;
  return (
    <section
      aria-labelledby="model-routing-heading"
      className="text-white"
      data-testid="model-routing-controls"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-base font-semibold" id="model-routing-heading">
            App-wide model routing
          </h2>
          <p className="mt-1 max-w-2xl text-sm text-slate-300">
            Applies to all managed agents and generated work. Local only never
            falls back to a cloud model.
          </p>
        </div>
        <fieldset className="flex flex-wrap gap-2">
          <legend className="sr-only">Adviser model routing</legend>
          <Button
            aria-pressed={preference === "cloud_first"}
            disabled={controlsDisabled}
            onClick={() => onChange("cloud_first")}
            size="sm"
            type="button"
            variant={preference === "cloud_first" ? "default" : "outline"}
          >
            <Cloud aria-hidden="true" />
            Cloud models first
          </Button>
          <Button
            aria-pressed={preference === "local_first"}
            disabled={controlsDisabled}
            onClick={() => onChange("local_first")}
            size="sm"
            type="button"
            variant={preference === "local_first" ? "default" : "outline"}
          >
            <Laptop aria-hidden="true" />
            Local model first
          </Button>
          <Button
            aria-pressed={preference === "local_only"}
            disabled={controlsDisabled}
            onClick={() => onChange("local_only")}
            size="sm"
            type="button"
            variant={preference === "local_only" ? "default" : "outline"}
          >
            <Unplug aria-hidden="true" />
            Local only
          </Button>
        </fieldset>
      </div>
      <p className="mt-2 text-xs text-slate-300">
        {preference === "cloud_first"
          ? "Selected route: cloud agent defaults. Briefs retain local fallback."
          : `Selected local runtime: LM Studio · gemma4-26b-official${
              preference === "local_only"
                ? " · cloud disabled"
                : " · briefs retain cloud fallback"
            }`}
      </p>
      {activeWork ? (
        <p className="mt-2 text-xs text-slate-300" role="status">
          Active agent work must finish before switching.
        </p>
      ) : null}
      {error ? (
        <p className="mt-2 text-sm text-destructive" role="alert">
          {error}
        </p>
      ) : null}
    </section>
  );
}
