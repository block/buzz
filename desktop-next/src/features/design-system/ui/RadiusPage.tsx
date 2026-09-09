import { FOUNDATION_GROUPS } from "@buzz/design-tokens/foundations";
import { PageHeader, Section } from "./primitives";
import { FoundationRoles } from "./FoundationRoles";
export function RadiusPage() {
  const group = FOUNDATION_GROUPS.find((group) => group.name === "Radius");
  if (!group) throw new Error("Missing foundation group");
  return (
    <>
      <PageHeader title={group.name} />
      <Section title="Corner roles">
        <FoundationRoles name="Radius" />
      </Section>
    </>
  );
}
