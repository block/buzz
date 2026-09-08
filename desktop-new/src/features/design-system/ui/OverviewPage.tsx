import { Link } from "@tanstack/react-router";

import { GRAMMAR } from "@/shared/tokens/registry";

import { PageHeader, Section } from "./primitives";

const LAYERS: Array<[string, string, string]> = [
  ["Layer 0", "a hue", "a theme package, or a person's preference"],
  ["Layer 1", "--accent-1…12   --neutral-1…12   --glass-1…5", "private ramps"],
  ["Layer 2", "--bg-purple-9: var(--accent-9)", "public roles, point at steps"],
  ["Layer 3", "bg-purple-9", "components, roles only"],
];

export function OverviewPage() {
  return (
    <>
      <PageHeader
        title="Buzz Design System"
        intro="A colour system small enough to hold in your head and precise enough that an agent picks the right value unsupervised. Structural colour is named and closed, because there are only a few right answers. Accent colour is a slot a theme or a person fills, because it should change without touching a component."
      />

      <Section
        title="The layers"
        description="Only the role layer is ever used when building a screen. That separation is what lets the entire look change without editing a component."
      >
        <div className="flex flex-col gap-2 rounded-xl bg-neutral-2 px-6 py-5">
          {LAYERS.map(([layer, what, why]) => (
            <div key={layer} className="flex flex-wrap items-baseline gap-x-4">
              <span className="w-16 shrink-0 text-body text-tertiary">
                {layer}
              </span>
              <code className="min-w-0 flex-1 text-body-sm text-primary">
                {what}
              </code>
              <span className="text-body-sm text-secondary">{why}</span>
            </div>
          ))}
        </div>
      </Section>

      <Section title="The grammar">
        <div className="rounded-lg bg-neutral-11 px-5 py-4">
          <code className="text-body text-neutral-1">{GRAMMAR}</code>
        </div>
        <p className="text-body-sm text-secondary">
          The order is fixed, so there is one correct spelling. The practical
          guidance for using and evolving it lives in{" "}
          <Link to="/design/maintaining" className="text-purple-12 underline">
            Maintaining the system
          </Link>
          .
        </p>
      </Section>

      <Section
        title="Where this stands"
        description="Colour and typography are complete and ready to build against. Spacing, radius, and motion are not started — their pages say what is still to decide rather than pretending to a system that does not exist yet. Components come one at a time, as the product repeats something."
      >
        <div className="flex flex-wrap gap-2">
          <Link
            to="/design/color"
            className="rounded-lg bg-purple-9 px-4 py-2 text-body text-on-accent transition-opacity hover:opacity-90"
          >
            Color
          </Link>
          <Link
            to="/design/typography"
            className="rounded-lg bg-neutral-2 px-4 py-2 text-body text-secondary transition-colors hover:bg-neutral-4 hover:text-primary"
          >
            Typography
          </Link>
          <Link
            to="/design/glass"
            className="rounded-lg bg-neutral-2 px-4 py-2 text-body text-secondary transition-colors hover:bg-neutral-4 hover:text-primary"
          >
            Glass
          </Link>
          <Link
            to="/design/maintaining"
            className="rounded-lg bg-neutral-2 px-4 py-2 text-body text-secondary transition-colors hover:bg-neutral-4 hover:text-primary"
          >
            Maintaining the system
          </Link>
        </div>
      </Section>
    </>
  );
}
