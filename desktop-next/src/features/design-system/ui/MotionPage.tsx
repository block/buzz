import { FOUNDATION_GROUPS } from "@buzz/design-tokens/foundations";
import { PageHeader, Section } from "./primitives";
import { FoundationRoles } from "./FoundationRoles";
import { PopoverDemo } from "../catalog/overlays";
import { ButtonsDemo } from "../catalog/forms";
export function MotionPage() {
  const group = FOUNDATION_GROUPS.find((group) => group.name === "Motion");
  if (!group) throw new Error("Missing foundation group");
  return (
    <>
      <PageHeader title={group.name} />
      <Section title="Try the feedback">
        <ButtonsDemo />
        <PopoverDemo />
      </Section>
      <Section title="Motion roles">
        <FoundationRoles name="Motion" />
      </Section>
    </>
  );
}
