import {
  TYPE_FAMILIES,
  TYPE_RAMPS,
  TYPE_ROLES,
  type TypeRole,
} from "@/shared/tokens/registry";

import { PageHeader, Row, Rows, Section, Specimens } from "./primitives";

/**
 * Every specimen below is set in the role it documents, so the page is the
 * system rather than a description of it. A role that reads badly here reads
 * badly in the product.
 */
function RoleSpecimen({ role }: { role: TypeRole }) {
  return (
    <div className="flex flex-col gap-2">
      <p
        className={`${role.token} ${role.mono ? "font-mono" : ""} text-primary`}
      >
        {role.mono ? "createChannel(name, members)" : "Bring your agents in"}
      </p>
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
        <code className="text-code text-accent">{role.token}</code>
        <span className="text-meta text-tertiary">{role.pointsAt}</span>
        <span className="text-meta text-tertiary">
          {role.size} / {role.lineHeight} / {role.tracking} / {role.weight}
        </span>
      </div>
    </div>
  );
}

export function TypographyPage() {
  return (
    <>
      <PageHeader title="Typography" />

      <Section title="The faces">
        <Specimens>
          {TYPE_FAMILIES.map((family) => (
            <div key={family.token} className="flex flex-col gap-1.5">
              <p
                className={`text-subheading text-primary ${
                  family.token === "font-mono" ? "font-mono" : "font-sans"
                }`}
              >
                {family.name}
              </p>
              <code className="text-code text-accent">{family.token}</code>
            </div>
          ))}
        </Specimens>
      </Section>

      <Section title="The roles">
        <Specimens>
          {TYPE_ROLES.map((role) => (
            <RoleSpecimen key={role.token} role={role} />
          ))}
        </Specimens>
      </Section>

      {TYPE_RAMPS.map((ramp) => (
        <Section key={ramp.id} title={ramp.name}>
          <Rows>
            {ramp.steps.map((step) => (
              <Row key={`${ramp.id}-${step.step}`}>
                <div className="flex flex-wrap items-baseline gap-x-4">
                  <code className="w-28 shrink-0 text-code text-primary">
                    {ramp.id} {step.step}
                  </code>
                  <span className="w-20 shrink-0 text-caption text-secondary">
                    {step.value}
                  </span>
                  <span className="text-caption text-tertiary">{step.job}</span>
                </div>
              </Row>
            ))}
          </Rows>
        </Section>
      ))}

      <Section title="Two rules">
        <Rows>
          <Row>
            <p className="text-label text-primary">
              Every size is relative. Never px.
            </p>
          </Row>
          <Row>
            <p className="text-label text-primary">
              No all-caps, and no tracked-out labels.
            </p>
          </Row>
        </Rows>
      </Section>
    </>
  );
}
