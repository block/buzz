import { Cloud, Laptop } from "lucide-react";

import type { ModelRoutingPreference } from "@/shared/api/tauriCommandBrief";
import { Button } from "@/shared/ui/button";

export function ModelRoutingControls({
  preference,
  disabled,
  error,
  onChange,
}: {
  preference: ModelRoutingPreference;
  disabled: boolean;
  error: string | null;
  onChange: (preference: ModelRoutingPreference) => void;
}) {
  return (
    <section
      aria-labelledby="model-routing-heading"
      className="text-white"
      data-testid="model-routing-controls"
    >
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <h2 className="text-base font-semibold" id="model-routing-heading">
            Adviser model routing
          </h2>
          <p className="mt-1 max-w-2xl text-sm text-slate-300">
            Choose the first provider for new briefs. If it is unavailable, the
            other route is tried automatically.
          </p>
        </div>
        <fieldset className="flex flex-wrap gap-2">
          <legend className="sr-only">Adviser model routing</legend>
          <Button
            aria-pressed={preference === "cloud_first"}
            disabled={disabled}
            onClick={() => onChange("cloud_first")}
            type="button"
            variant={preference === "cloud_first" ? "default" : "outline"}
          >
            <Cloud aria-hidden="true" />
            Cloud models first
          </Button>
          <Button
            aria-pressed={preference === "local_first"}
            disabled={disabled}
            onClick={() => onChange("local_first")}
            type="button"
            variant={preference === "local_first" ? "default" : "outline"}
          >
            <Laptop aria-hidden="true" />
            Local model first
          </Button>
        </fieldset>
      </div>
      <p className="mt-3 text-sm text-slate-400">
        The selection applies to the next run; an active brief keeps the route
        it started with.
      </p>
      {error ? (
        <p className="mt-2 text-sm text-destructive" role="alert">
          {error}
        </p>
      ) : null}
    </section>
  );
}
