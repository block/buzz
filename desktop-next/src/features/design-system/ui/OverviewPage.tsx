import { Link } from "@tanstack/react-router";

import { GRAMMAR } from "@/shared/tokens/registry";

import { PageHeader, Section } from "./primitives";

const LAYERS: Array<[string, string, string]> = [
  ["Layer 0", "a hue", "a theme package, or a person's preference"],
  ["Layer 1", "--accent-1…12   --neutral-1…12   --glass-1…5", "private ramps"],
  ["Layer 2", "--bg-accent: var(--accent-9)", "public roles, point at steps"],
  ["Layer 3", "bg-accent", "components, roles only"],
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
        <div className="flex flex-col gap-2 rounded-xl bg-inset px-6 py-5">
          {LAYERS.map(([layer, what, why]) => (
            <div key={layer} className="flex flex-wrap items-baseline gap-x-4">
              <span className="w-16 shrink-0 text-label text-tertiary">
                {layer}
              </span>
              <code className="min-w-0 flex-1 text-caption text-primary">
                {what}
              </code>
              <span className="text-caption text-secondary">{why}</span>
            </div>
          ))}
        </div>
      </Section>

      <Section title="The grammar">
        <div className="rounded-lg bg-inverse px-5 py-4">
          <code className="text-label text-on-inverse">{GRAMMAR}</code>
        </div>
        <p className="text-caption text-secondary">
          The order is fixed, so there is one correct spelling. See{" "}
          <Link to="/design/vocabulary" className="text-accent underline">
            the vocabulary
          </Link>{" "}
          for every word a token can be built from.
        </p>
      </Section>

      <Section
        title="Where this stands"
        description="Colour and typography are complete and ready to build against. Spacing, radius, and motion are not started — their pages say what is still to decide rather than pretending to a system that does not exist yet. Components come one at a time, as the product repeats something."
      >
        <div className="flex flex-wrap gap-2">
          <Link
            to="/design/color"
            className="rounded-lg bg-accent px-4 py-2 text-label text-on-accent transition-opacity hover:opacity-90"
          >
            Color
          </Link>
          <Link
            to="/design/typography"
            className="rounded-lg bg-inset px-4 py-2 text-body text-secondary transition-colors hover:bg-hover hover:text-primary"
          >
            Typography
          </Link>
          <Link
            to="/design/glass"
            className="rounded-lg bg-inset px-4 py-2 text-body text-secondary transition-colors hover:bg-hover hover:text-primary"
          >
            Glass
          </Link>
          <Link
            to="/design/growth"
            className="rounded-lg bg-inset px-4 py-2 text-body text-secondary transition-colors hover:bg-hover hover:text-primary"
          >
            Growing the system
          </Link>
        </div>
      </Section>
    </>
  );
}
