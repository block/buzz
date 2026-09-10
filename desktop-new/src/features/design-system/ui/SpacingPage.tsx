import { SPACE, SPACE_ROLES } from "@/shared/tokens/registry";
import { FoundationScale } from "./FoundationScale";
import { PageHeader, Section } from "./primitives";

export function SpacingPage() {
  return (
    <>
      <PageHeader
        title="Spacing"
        status="forming"
        intro="A short six-step rhythm extracted from the Messages frame. The first roles capture only the relationships repeated by the workspace and its dense navigator; new roles arrive when another real component earns them."
      />

      <Section
        title="Scale"
        description="Components use these steps through roles where the distance carries product meaning."
      >
        <FoundationScale
          items={SPACE.map((item) => ({
            token: `space ${item.step}`,
            value: item.value,
            use: item.use,
          }))}
        />
      </Section>

      <Section title="Roles">
        <FoundationScale
          items={SPACE_ROLES.map((item) => ({
            token: item.token,
            value: item.pointsAt,
            use: item.use,
          }))}
        />
      </Section>
    </>
  );
}
