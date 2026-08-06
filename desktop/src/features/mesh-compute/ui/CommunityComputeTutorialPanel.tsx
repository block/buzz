import { ArrowLeft, ArrowRight, FlaskConical, X } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import type { CommunityComputeTutorialFixtureId } from "../communityComputeFixtures";

const STEPS = [
  {
    title: "Every territory is a model deployment",
    body: "Each connected group is one ready model on one contributed device. If one device serves multiple models, each model gets its own territory; another device serving the same model appears separately.",
  },
  {
    title: "Area represents memory",
    body: "Models snap to four coarse footprints: 1, 2, 4, or 7 cells. The footprint comes from reported model size, or shared device capacity when model size is unavailable.",
  },
  {
    title: "Motion illustrates inference in flight",
    body: "For this tutorial, breathing territories simulate models handling requests. The live community snapshot cannot yet identify the deployment serving a request, so the live map remains static.",
  },
  {
    title: "The headline measures the pool",
    body: "Members sharing counts contributors, contributed VRAM is the memory they offered, and VRAM allocated is the reported model footprint—not GPU processing utilization.",
  },
  {
    title: "Health summarizes reported state",
    body: "The scorecard counts ready, warming, and idle deployments from current status reports. It does not infer utilization, offline devices, replica resilience, or capacity headroom.",
  },
] as const;

export function CommunityComputeTutorialPanel({
  fixtureId,
  onExit,
  onFixtureChange,
  onStepChange,
  step,
}: {
  fixtureId: CommunityComputeTutorialFixtureId;
  onExit: () => void;
  onFixtureChange: (fixture: CommunityComputeTutorialFixtureId) => void;
  onStepChange: (step: number) => void;
  step: number;
}) {
  const current = STEPS[step] ?? STEPS[0];
  const showDeveloperControls =
    import.meta.env.DEV || import.meta.env.MODE === "e2e";

  return (
    <section
      aria-labelledby="mesh-tutorial-title"
      className="rounded-2xl border border-foreground/20 bg-foreground/[0.035] p-4"
      data-testid="community-compute-tutorial-panel"
    >
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
            Tutorial · {step + 1} of {STEPS.length}
          </p>
          <h2 className="mt-1 text-base font-semibold" id="mesh-tutorial-title">
            {current.title}
          </h2>
          <p className="mt-1 max-w-3xl text-sm leading-relaxed text-muted-foreground">
            {current.body}
          </p>
        </div>
        <Button
          aria-label="Exit mesh tutorial"
          onClick={onExit}
          size="icon-xs"
          type="button"
          variant="ghost"
        >
          <X />
        </Button>
      </div>

      <div className="mt-4 flex flex-wrap items-center gap-2">
        <Button
          disabled={step === 0}
          onClick={() => onStepChange(step - 1)}
          size="sm"
          type="button"
          variant="outline"
        >
          <ArrowLeft /> Back
        </Button>
        <Button
          onClick={() =>
            step === STEPS.length - 1 ? onStepChange(0) : onStepChange(step + 1)
          }
          size="sm"
          type="button"
        >
          {step === STEPS.length - 1 ? "Replay" : "Next"}
          <ArrowRight />
        </Button>
        <fieldset className="ml-1 flex gap-1" aria-label="Tutorial progress">
          {STEPS.map((item, index) => (
            <button
              aria-label={`Go to tutorial step ${index + 1}: ${item.title}`}
              className={cn(
                "size-2 rounded-full bg-muted-foreground/25 transition-colors",
                index === step && "bg-foreground",
              )}
              key={item.title}
              onClick={() => onStepChange(index)}
              type="button"
            />
          ))}
        </fieldset>
      </div>

      {showDeveloperControls ? (
        <div
          className="mt-4 flex flex-wrap items-center gap-2 border-border/60 border-t pt-3"
          data-testid="community-compute-dev-controls"
        >
          <FlaskConical className="size-3.5 text-muted-foreground" />
          <label
            className="text-xs font-medium"
            htmlFor="mesh-tutorial-scenario"
          >
            Preview scenario
          </label>
          <select
            className="h-8 rounded-lg border border-input/50 bg-background px-2 text-xs focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            id="mesh-tutorial-scenario"
            onChange={(event) =>
              onFixtureChange(
                event.target.value as CommunityComputeTutorialFixtureId,
              )
            }
            value={fixtureId}
          >
            <option value="empty">Empty community</option>
            <option value="single-device">Single device</option>
            <option value="small-company">Small company</option>
            <option value="large-dense-500">500 deployments</option>
          </select>
          <span className="text-2xs text-muted-foreground">
            Development-only · deterministic frontend data
          </span>
        </div>
      ) : null}
    </section>
  );
}
