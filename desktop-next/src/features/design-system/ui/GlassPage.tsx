import { Tabs } from "@buzz/ui";
import { BLUR, RAMPS } from "@/shared/tokens/registry";

import { PageHeader, Section, Swatch } from "./primitives";

const GLASS = RAMPS.find((ramp) => ramp.id === "glass");

function ChromePill() {
  const items = ["Me", "Messages", "Projects"];
  return (
    <Tabs.Root defaultValue="Messages">
      <Tabs.List variant="glass" aria-label="Glass navigation preview">
        {items.map((label) => (
          <Tabs.Tab key={label} value={label}>
            {label}
          </Tabs.Tab>
        ))}
        <Tabs.Indicator />
      </Tabs.List>
      {items.map((label) => (
        <Tabs.Panel
          key={label}
          value={label}
          tabIndex={-1}
          className="bui-sr-only"
        >
          {label} surface selected in this material preview.
        </Tabs.Panel>
      ))}
    </Tabs.Root>
  );
}

export function GlassPage() {
  return (
    <>
      <PageHeader title="Glass" />

      <Section title="Live">
        <div className="texture-dots flex items-center justify-center rounded-xl bg-app px-8 py-12">
          <ChromePill />
        </div>
        <div className="flex flex-col gap-1 text-caption text-secondary">
          <span>
            container <code className="text-primary">bg-chrome-glass</code> +{" "}
            <code className="text-primary">rim-glass</code> +{" "}
            <code className="text-primary">blur-chrome</code>
          </span>
          <span>
            selected <code className="text-primary">bg-chrome-selected</code>
          </span>
        </div>
      </Section>

      {GLASS ? (
        <Section title="The glass ramp">
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

      <Section title="Blur">
        <div className="flex flex-wrap gap-3 rounded-xl bg-app p-4">
          {BLUR.map((blur) => (
            <div
              key={blur.token}
              className="flex flex-col items-center gap-2 rounded-lg bg-chrome-glass px-6 py-5"
              style={{ backdropFilter: `blur(${blur.value})` }}
            >
              <code className="text-meta text-primary">{blur.token}</code>
              <span className="text-caption text-tertiary">{blur.value}</span>
            </div>
          ))}
        </div>
      </Section>

      <Section title="The rim">
        <div className="rounded-lg bg-inverse px-5 py-4">
          <code className="whitespace-pre text-caption text-on-inverse">
            {`box-shadow:\n  inset 0  1px 0 var(--rim-lit),\n  inset 0 -1px 0 var(--rim-shade);`}
          </code>
        </div>
      </Section>
    </>
  );
}
