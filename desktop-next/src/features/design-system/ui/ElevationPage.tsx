import { ELEVATION } from "@/shared/tokens/registry";

import { Note, PageHeader, Section } from "./primitives";

export function ElevationPage() {
  return (
    <>
      <PageHeader
        title="Elevation"
        intro="Two values, taken from the design exploration unchanged. Depth comes from hairline borders and shadows at the threshold of perception rather than from many surface colours — if a shadow is obvious, it is too strong."
      />

      <Section title="The values">
        <div className="flex flex-wrap gap-6 rounded-xl bg-app p-8">
          {ELEVATION.map((level) => (
            <div key={level.token} className="flex flex-col gap-2">
              <div
                className="flex h-24 w-44 items-center justify-center rounded-xl bg-panel"
                style={{ boxShadow: `var(${level.variable})` }}
              >
                <code className="text-body-sm text-primary">{level.token}</code>
              </div>
              <span className="max-w-44 text-body-sm text-secondary">
                {level.use}
              </span>
            </div>
          ))}
        </div>
      </Section>

      <Note>
        Elevation is carried by shadow in light mode and by lightness in dark
        mode. On a near-black background there is nothing darker for a shadow to
        cast, so a floating surface becomes a step lighter instead — which is
        why `bg-float` and `bg-panel` share a light value and diverge in dark.
        Never reach for a stronger shadow to make something float in dark mode.
      </Note>
    </>
  );
}
