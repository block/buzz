import {
  EXCEPTIONS,
  RAMPS,
  ROLE_GROUPS,
  type Role,
} from "@/shared/tokens/registry";

import { PageHeader, Section, Swatch } from "./primitives";

/**
 * A role, sitting directly on the page.
 *
 * No card and no divider: the swatch is its own separator, and a colour judged
 * on a grey card is not being judged on the surface it will actually be used on.
 * The hairline stays on the swatch itself — `bg-panel` is white on a white page,
 * so without it the most-used role in the system renders as nothing. That is a
 * genuine boundary rather than decoration.
 */
function RoleRow({ role }: { role: Role }) {
  return (
    <div className="flex items-start gap-4 py-2.5">
      <div
        className="mt-0.5 h-9 w-16 shrink-0 rounded-md border border-tertiary"
        style={{ background: `var(${role.variable})` }}
      />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <code className="text-label text-primary">{role.token}</code>
          <span className="text-caption text-tertiary">{role.pointsAt}</span>
          {role.status !== "core" ? (
            <span className="rounded-full bg-warning-tint px-2 py-0.5 text-meta text-warning">
              {role.status}
              {role.owner ? ` · ${role.owner}` : ""}
            </span>
          ) : null}
        </div>
        {role.exception ? (
          <p className="text-caption text-tertiary">
            Exception: {role.exception}
          </p>
        ) : null}
      </div>
    </div>
  );
}

export function ColourPage() {
  return (
    <>
      <PageHeader title="Colour" />

      <Section title="Layer 1 — private ramps">
        <div className="flex flex-col gap-8">
          {RAMPS.map((ramp) => (
            <div key={ramp.id} className="flex flex-col gap-3">
              <div className="flex flex-col gap-1">
                <h3 className="text-label text-primary">{ramp.name}</h3>
              </div>
              <div
                className={`grid gap-3 ${
                  ramp.steps.length > 6
                    ? "grid-cols-4 sm:grid-cols-6"
                    : "grid-cols-3 sm:grid-cols-5"
                } ${ramp.translucent ? "rounded-xl bg-app p-4" : ""}`}
              >
                {ramp.steps.map((step) => (
                  <Swatch
                    key={step.variable}
                    variable={step.variable}
                    label={`${ramp.id} ${step.step}`}
                    sublabel={step.job}
                    translucent={ramp.translucent}
                  />
                ))}
              </div>
            </div>
          ))}
        </div>
      </Section>

      <Section title="Layer 2 — public roles">
        <div className="flex flex-col gap-8">
          {ROLE_GROUPS.map((group) => (
            <div key={group.id} className="flex flex-col gap-2">
              <div className="flex flex-col gap-1">
                <h3 className="text-label text-primary">{group.name}</h3>
              </div>
              <div className="mt-1 flex flex-col">
                {group.roles.map((role) => (
                  <RoleRow key={role.token} role={role} />
                ))}
              </div>
            </div>
          ))}
        </div>
      </Section>

      <Section title="Deliberate exceptions">
        {/* No swatch here to do the separating, so these entries keep a little
            structure — the token name leads and the spacing groups it with its
            reason. Still no card: this is prose, not data. */}
        <div className="flex flex-col gap-5">
          {EXCEPTIONS.map((exception) => (
            <div key={exception.name} className="flex flex-col gap-1">
              <code className="text-code text-accent">{exception.name}</code>
              <p className="max-w-2xl text-caption text-secondary">
                {exception.why}
              </p>
            </div>
          ))}
        </div>
      </Section>
    </>
  );
}
