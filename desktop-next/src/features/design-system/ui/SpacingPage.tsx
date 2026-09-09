import { FOUNDATION_GROUPS } from "@buzz/design-tokens/foundations";
import { PageHeader, Section } from "./primitives";
import { FoundationRoles } from "./FoundationRoles";
export function SpacingPage() {
  const group = FOUNDATION_GROUPS.find((group) => group.name === "Spacing");
  if (!group) throw new Error("Missing foundation group");
  return (
    <>
      <PageHeader title={group.name} />
      <Section title="Spacing roles">
        <FoundationRoles name="Spacing" />
      </Section>
      <Section title="Controls and layers">
        <FoundationRoles name="Controls" />
      </Section>
    </>
  );
}
