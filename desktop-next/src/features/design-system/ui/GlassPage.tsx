import { BLUR, RAMPS } from "@/shared/tokens/registry";

import { Note, PageHeader, Section, Swatch } from "./primitives";

const GLASS = RAMPS.find((ramp) => ramp.id === "glass");

function ChromePill() {
  const items: Array<[string, boolean]> = [
    ["Me", false],
    ["Messages", true],
    ["Projects", false],
  ];
  return (
    <div className="rim-glass blur-chrome flex items-center gap-1 rounded-full bg-chrome-glass p-1.5">
      {items.map(([label, selected]) => (
        <button
          key={label}
          type="button"
          className={
            selected
              ? "elevate-xs rounded-full bg-chrome-selected px-5 py-2 text-body text-primary"
              : "rounded-full px-5 py-2 text-body text-secondary transition-colors hover:bg-chrome-glass-hover hover:text-primary"
          }
        >
          {label}
        </button>
      ))}
    </div>
  );
}

export function GlassPage() {
  return (
    <>
      <PageHeader
        title="Glass"
        intro="A translucent blurred surface is the same region in a different material, so it is named as that region plus its material rather than assembled per screen from a fill, a transparency, and a blur amount. Translucency and blur live inside the value."
      />

      <Section
        title="Live"
        description="The chrome pill, rendered from the real tokens over the real backdrop. Hover an unselected item to see the glass hover move one step up the ramp."
      >
        <div className="texture-dots flex items-center justify-center rounded-xl bg-app px-8 py-12">
          <ChromePill />
        </div>
        <div className="flex flex-col gap-1 text-body-sm text-secondary">
          <span>
            container <code className="text-primary">bg-chrome-glass</code> +{" "}
            <code className="text-primary">rim-glass</code> +{" "}
            <code className="text-primary">blur-chrome</code>
          </span>
          <span>
            selected <code className="text-primary">bg-chrome-selected</code> —
            opaque where its container is glass
          </span>
        </div>
      </Section>

      {GLASS ? (
        <Section title="The glass ramp" description={GLASS.description}>
          <div className="grid grid-cols-3 gap-3 rounded-xl bg-app p-4 sm:grid-cols-5">
            {GLASS.steps.map((step) => (
              <Swatch
                key={step.variable}
                variable={step.variable}
                label={`glass ${step.step}`}
                sublabel={step.job}
                translucent
              />
            ))}
          </div>
        </Section>
      ) : null}

      <Section
        title="Blur"
        description="A separate axis from translucency: a glass role names one step on the glass ramp and one blur amount, so a quiet glass can still be heavily blurred."
      >
        <div className="flex flex-wrap gap-3 rounded-xl bg-app p-4">
          {BLUR.map((blur) => (
            <div
              key={blur.token}
              className="flex flex-col items-center gap-2 rounded-lg bg-chrome-glass px-6 py-5"
              style={{ backdropFilter: `blur(${blur.value})` }}
            >
              <code className="text-body-sm text-primary">{blur.token}</code>
              <span className="text-body-sm text-tertiary">{blur.value}</span>
            </div>
          ))}
        </div>
      </Section>

      <Section
        title="The rim"
        description="A deliberate exception: two literal values, not a ramp step and not one of the numbered gradients. Those are background treatments; this is a material detail. Real glass catches light along one edge and falls away on the opposite one, so the rim is a directional pair from one fixed light direction that every glass surface shares."
      >
        <div className="rounded-lg bg-inverse px-5 py-4">
          <code className="whitespace-pre text-body-sm text-on-inverse">
            {`box-shadow:\n  inset 0  1px 0 var(--rim-lit),\n  inset 0 -1px 0 var(--rim-shade);`}
          </code>
        </div>
        <Note>
          CSS borders accept only solid colours, so every gradient-border
          technique is a workaround. `border-image` takes a gradient but ignores
          `border-radius`, which is fatal on a pill. The two-background
          `background-clip` trick respects radius but needs an opaque inner
          fill, so it bleeds across a translucent surface. Two inset shadows
          respect radius, cost nothing, and work over translucency. The known
          limit is that each shadow is one solid colour, so a corner transitions
          in two discrete edges rather than a smooth sweep.
        </Note>
      </Section>

      <Note>
        A glass hover changes opacity and never blur — re-blurring a large
        surface every frame is expensive enough to feel. Glass surfaces use
        their own `-glass-hover` rather than the shared `bg-hover`, which is a
        neutral built to sit on an opaque surface.
      </Note>
    </>
  );
}
