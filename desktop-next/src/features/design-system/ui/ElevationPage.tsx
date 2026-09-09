import { ELEVATION } from "@/shared/tokens/registry";

import { PageHeader, Section } from "./primitives";

export function ElevationPage() {
  return (
    <>
      <PageHeader title="Elevation" />

      <Section title="The values">
        <div className="flex flex-wrap gap-6 rounded-xl bg-app p-8">
          {ELEVATION.map((level) => (
            <div key={level.token} className="flex flex-col gap-2">
              <div
                className="flex h-24 w-44 items-center justify-center rounded-xl bg-panel"
                style={{ boxShadow: `var(${level.variable})` }}
              >
                <code className="text-meta text-primary">{level.token}</code>
              </div>
              <span className="max-w-44 text-caption text-secondary">
                {level.use}
              </span>
            </div>
          ))}
        </div>
      </Section>
    </>
  );
}
