import { FOUNDATION_GROUPS } from "@buzz/design-tokens/foundations";
import { PageHeader, Section } from "./primitives";
export function SpacingPage() {
  const group = FOUNDATION_GROUPS[0];
  return (
    <>
      <PageHeader title={group.name} intro={group.description} />
      <Section title="Roles">
        <dl className="grid gap-6">
          {group.roles.map(([name, value, use]) => (
            <div key={name} className="grid gap-2 sm:grid-cols-3">
              <dt className="text-code font-mono">{name}</dt>
              <dd className="text-body text-secondary">{value}</dd>
              <dd className="text-caption text-secondary">{use}</dd>
            </div>
          ))}
        </dl>
      </Section>
    </>
  );
}
