import { RADII } from "@/shared/tokens/registry";
import { FoundationScale } from "./FoundationScale";
import { PageHeader, Section } from "./primitives";

export function RadiusPage() {
  return (
    <>
      <PageHeader
        title="Radius"
        status="forming"
        intro="Four corner roles reproduce the Messages frame without turning every measured curve into a choice: dense rows, controls, major panels, and fully round identity or chrome."
      />

      <Section title="Roles">
        <FoundationScale
          items={RADII.map((item) => ({
            token: item.token,
            value: item.value,
            use: item.use,
          }))}
        />
      </Section>

      <Section title="Relationship">
        <div className="component-radius-demo bg-app">
          {RADII.map((item) => (
            <div
              key={item.token}
              className="component-radius-sample bg-panel text-body-sm text-secondary"
              style={{ borderRadius: `var(${item.variable})` }}
            >
              {item.token}
            </div>
          ))}
        </div>
      </Section>
    </>
  );
}
