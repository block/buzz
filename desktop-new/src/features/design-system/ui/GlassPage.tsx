import { RAMPS } from "@/shared/tokens/registry";

import { PageHeader, Section } from "./primitives";

const GLASS = RAMPS.find((ramp) => ramp.id === "glass");

const MATERIALS = [
  {
    utility: "glass-primary",
    spec: "glass-2 · blur-md · rim",
    use: "On the backdrop",
  },
  {
    utility: "glass-secondary",
    spec: "glass-4 · blur-lg · rim · shadow-sm",
    use: "Over glass",
  },
];

/**
 * A material, rendered as itself over the backdrop.
 *
 * The label sits inside the specimen rather than in a caption beneath it: the
 * subject is a translucent surface, so text on it is part of what is being
 * judged — whether it stays legible with the gradient reading through.
 */
function Material({ material }: { material: (typeof MATERIALS)[number] }) {
  return (
    <div
      className={`${material.utility} flex min-h-[8rem] flex-col justify-between gap-6 rounded-xl px-5 py-4`}
    >
      <span className="text-body-sm text-secondary">{material.use}</span>
      <div className="flex flex-col gap-1">
        <code className="text-mono text-primary">{material.utility}</code>
        <span className="text-body-sm text-tertiary">{material.spec}</span>
      </div>
    </div>
  );
}

export function GlassPage() {
  return (
    <>
      <PageHeader
        title="Glass"
        intro="Two materials, named by how high they sit. Each one is a fill, a blur, a rim, and sometimes a shadow, applied as a single utility so it cannot arrive in pieces."
      />

      <Section
        title="The materials"
        description="Over the app backdrop, because translucency can only be judged against what shows through it."
      >
        <div className="glass-scene grid gap-4 rounded-xl p-6 sm:grid-cols-2">
          {MATERIALS.map((material) => (
            <Material key={material.utility} material={material} />
          ))}
        </div>
      </Section>

      <Section
        title="Interactive"
        description="The same materials with a hover, for glass you can click. Hover moves one step up the ramp and never changes blur."
      >
        <div className="glass-scene flex flex-wrap gap-3 rounded-xl p-6">
          <button
            type="button"
            className="glass-primary-interactive rounded-full px-5 py-2.5 text-body text-primary transition-colors"
          >
            glass-primary-interactive
          </button>
          <button
            type="button"
            className="glass-secondary-interactive rounded-full px-5 py-2.5 text-body text-primary transition-colors"
          >
            glass-secondary-interactive
          </button>
        </div>
      </Section>

      {GLASS ? (
        <Section
          title="The ramp"
          description="Each step is the mode's own surface colour at an increasing opacity, which is what lets a hover move one step up instead of holding its own literal. Fills only — the rim is separate."
        >
          {/* One continuous strip rather than five separate swatches: the
              subject is a progression, and gaps between cards let the backdrop
              re-enter between them so each step is read against a different
              part of the gradient instead of against its neighbours.
              Deliberately not the shared `Swatch` — its hairline border reads
              as the glass rim on a translucent fill. */}
          <div className="glass-scene rounded-xl p-6">
            <div className="flex overflow-hidden rounded-xl">
              {GLASS.steps.map((step) => (
                <div
                  key={step.variable}
                  className="blur-chrome flex min-w-0 flex-1 flex-col justify-end gap-1 px-3 py-4"
                  style={{ background: `var(${step.variable})` }}
                >
                  <code className="truncate text-mono-sm text-primary">
                    glass {step.step}
                  </code>
                  <span className="truncate text-body-sm text-tertiary">
                    {step.job}
                  </span>
                </div>
              ))}
            </div>
          </div>
        </Section>
      ) : null}

      <Section
        title="The rim"
        description="Real glass catches light along one edge and falls away on the opposite one, so the rim is a directional pair sharing one fixed light direction. Two inset shadows rather than a border, because a CSS border cannot hold a gradient and keep its radius."
      >
        <div className="glass-scene rounded-xl p-6">
          <div className="glass-primary rounded-xl px-5 py-4">
            <code className="whitespace-pre text-mono-sm text-primary">
              {`inset 0  1px 0 var(--rim-lit)\ninset 0 -1px 0 var(--rim-shade)`}
            </code>
          </div>
        </div>
      </Section>
    </>
  );
}
