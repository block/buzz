import {
  TYPE_FAMILIES,
  TYPE_RAMPS,
  TYPE_ROLES,
  type TypeRole,
} from "@/shared/tokens/registry";

import { Note, PageHeader, Row, Rows, Section, Specimens } from "./primitives";

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
        <code className="text-mono text-accent">{role.token}</code>
        <span className="text-body-sm text-tertiary">{role.pointsAt}</span>
        <span className="text-body-sm text-tertiary">
          {role.size} / {role.lineHeight} / {role.tracking} / {role.weight}
        </span>
      </div>
      <p className="max-w-xl text-body-sm text-secondary">{role.use}</p>
    </div>
  );
}

export function TypographyPage() {
  return (
    <>
      <PageHeader
        title="Typography"
        intro="Nine sizes, ten roles, two faces. A role carries its whole setting — size, line height, letter spacing, weight — because those four are one decision rather than four, so text-body alone produces correctly set text and there is nothing left to get wrong."
      />

      <Section
        title="The faces"
        description="Both already ship in every current Buzz client, so this is a decision to keep rather than to make. Inter is drawn for interface text at small sizes, which is most of this product."
      >
        <Specimens>
          {TYPE_FAMILIES.map((family) => (
            <div key={family.token} className="flex flex-col gap-1.5">
              <p
                className={`text-heading text-primary ${
                  family.token === "font-mono" ? "font-mono" : "font-sans"
                }`}
              >
                {family.name}
              </p>
              <code className="text-mono text-accent">{family.token}</code>
              <p className="max-w-xl text-body-sm text-secondary">
                {family.use}
              </p>
            </div>
          ))}
        </Specimens>
      </Section>

      <Section
        title="The roles"
        description="Named for the job the text does, never for its size. text-title, not text-28 — a size name is a value in disguise and goes stale the moment the ramp moves. Each specimen is set in the role it documents."
      >
        <Specimens>
          {TYPE_ROLES.map((role) => (
            <RoleSpecimen key={role.token} role={role} />
          ))}
        </Specimens>
      </Section>

      {TYPE_RAMPS.map((ramp) => (
        <Section key={ramp.id} title={ramp.name} description={ramp.description}>
          <Rows>
            {ramp.steps.map((step) => (
              <Row key={`${ramp.id}-${step.step}`}>
                <div className="flex flex-wrap items-baseline gap-x-4">
                  <code className="w-28 shrink-0 text-mono text-primary">
                    {ramp.id} {step.step}
                  </code>
                  <span className="w-20 shrink-0 text-body-sm text-secondary">
                    {step.value}
                  </span>
                  <span className="text-body-sm text-tertiary">{step.job}</span>
                </div>
              </Row>
            ))}
          </Rows>
        </Section>
      ))}

      <Section
        title="Two rules"
        description="Both are inherited rather than invented — the existing client learned each of them the expensive way."
      >
        <Rows>
          <Row>
            <p className="text-body text-primary">
              Every size is relative. Never px.
            </p>
            <p className="mt-1.5 max-w-xl text-body-sm text-secondary">
              Fixed pixel text freezes against keyboard zoom and ignores the
              person's font-size preference. The current client shipped a
              message-timeline regression from exactly this and now has a CI
              guard rejecting arbitrary size literals. This ramp derives
              entirely from one virtual rem, so both dials work by construction.
            </p>
          </Row>
          <Row>
            <p className="text-body text-primary">
              No all-caps, and no tracked-out labels.
            </p>
            <p className="mt-1.5 max-w-xl text-body-sm text-secondary">
              A capitalised label is harder to read than the sentence-case
              version and reads as enterprise chrome. Section labels earn their
              quietness from size and colour — text-body-sm on text-tertiary —
              rather than from being shouted. There is no uppercase utility in
              this system.
            </p>
          </Row>
        </Rows>
      </Section>

      <Note>
        The Cash Sans and BlockUI type variables that appear in the design
        exploration are contamination from another Figma library linked into
        that file. They exist nowhere in any Buzz codebase and need no cleanup —
        only a decision not to inherit them.
      </Note>
    </>
  );
}
