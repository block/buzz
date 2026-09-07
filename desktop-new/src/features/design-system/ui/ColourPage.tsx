import {
  BACKDROP_CHOICES,
  BACKDROP_TREATMENTS,
  EXCEPTIONS,
  PALETTE,
  RAMPS,
  ROLE_GROUPS,
  type BackdropTreatment,
  type Role,
} from "@/shared/tokens/registry";

import { Note, PageHeader, Section, Swatch } from "./primitives";

/**
 * A role, sitting directly on the page.
 *
 * No card and no divider: the swatch is its own separator, and a colour judged
 * on a grey card is not being judged on the surface it will actually be used on.
 * The hairline stays on the swatch itself — `bg-panel` is white on a white page,
 * so without it the most-used role in the system renders as nothing. That is a
 * genuine boundary rather than decoration.
 */
function BackdropTreatmentRow({ treatment }: { treatment: BackdropTreatment }) {
  const paired = BACKDROP_TREATMENTS.find(
    (candidate) => candidate.id === treatment.pairedWith,
  );

  return (
    <div className="flex items-start gap-4 py-2.5">
      <div
        className="mt-0.5 h-9 w-16 shrink-0 rounded-md border border-primary"
        style={{ background: `var(${treatment.variable})` }}
      />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <code className="text-body text-primary">{treatment.name}</code>
          <span className="text-body-sm text-tertiary">
            {treatment.mode} only
          </span>
        </div>
        <p className="text-body-sm text-secondary">{treatment.description}</p>
        <p className="text-body-sm text-tertiary">
          Pairs with {paired?.name ?? treatment.pairedWith} in {paired?.mode}
        </p>
      </div>
    </div>
  );
}

function RoleRow({ role }: { role: Role }) {
  return (
    <div className="flex items-start gap-4 py-2.5">
      <div
        className="mt-0.5 h-9 w-16 shrink-0 rounded-md border border-primary"
        style={{ background: `var(${role.variable})` }}
      />
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <code className="text-body text-primary">{role.token}</code>
          <span className="text-body-sm text-tertiary">{role.pointsAt}</span>
          {role.status !== "core" ? (
            <span className="rounded-full bg-purple-3 px-2 py-0.5 text-body-sm text-purple-12">
              {role.status}
              {role.owner ? ` · ${role.owner}` : ""}
            </span>
          ) : null}
        </div>
        <p className="text-body-sm text-secondary">{role.use}</p>
        {role.exception ? (
          <p className="text-body-sm text-tertiary">
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
      <PageHeader
        title="Colour"
        intro="Four layers, and only the role layer is used when building a screen. The palette holds raw values — colour ramps and named backdrop treatments. Semantic tokens hold stable choices. Roles hold interface meanings. Everything below is rendered from the token registry, so a token added there appears here automatically and this page cannot drift from the system."
      />

      <Section
        title="Layer 0 — the palette"
        description="Every hue, twelve steps, authored per mode — the only place a literal colour lives. It exists because the layer above it was 114 hand-picked values with nothing keeping two tokens that do the same job in agreement, and they drifted. Dark steps are authored for dark surfaces rather than derived by dimming light ones, so a subtler deep colour is a step you pick instead of an opacity you write."
      >
        <div className="flex flex-col gap-6">
          {PALETTE.map((hue) => (
            <div key={hue.id} className="flex flex-col gap-2">
              <div className="flex flex-wrap items-baseline gap-2">
                <h3 className="text-body text-primary">{hue.id}</h3>
                <span className="text-body-sm text-tertiary">
                  {hue.usedBy ? `drawn from by ${hue.usedBy}` : "unassigned"}
                </span>
              </div>
              <div className="flex gap-1">
                {hue.steps.map((step) => (
                  <div
                    key={step.variable}
                    className="flex min-w-0 flex-1 flex-col gap-1"
                  >
                    <div
                      className="h-10 rounded-md border border-primary"
                      style={{ background: `var(${step.variable})` }}
                    />
                    <span className="text-center text-body-sm text-tertiary">
                      {step.step}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      </Section>

      <Section
        title="Layer 0 — backdrop treatments"
        description="These are named visual compositions, not colour ramps: each holds the exact colours, geometry, falloff, and sometimes a vignette that make a scene. A treatment exists only in the mode where its name is true; its paired counterpart is a different named scene, not a dimmed copy. Components never reference these."
      >
        <div className="flex flex-col gap-6">
          {(["light", "dark"] as const).map((mode) => (
            <div key={mode} className="flex flex-col gap-2">
              <h3 className="text-body text-primary">{mode} mode</h3>
              <div className="flex flex-col">
                {BACKDROP_TREATMENTS.filter(
                  (treatment) => treatment.mode === mode,
                ).map((treatment) => (
                  <BackdropTreatmentRow
                    key={treatment.id}
                    treatment={treatment}
                  />
                ))}
              </div>
            </div>
          ))}
        </div>
      </Section>

      <Section
        title="Layer 1 — semantic backdrop choices"
        description="A numbered choice is the stable cross-mode selection: choose one slot and its named light and dark treatments travel together. An appearance setting selects gradient-1 through gradient-4; product surfaces still use bg-app."
      >
        <div className="flex flex-col">
          {BACKDROP_CHOICES.map((choice) => {
            const light = BACKDROP_TREATMENTS.find(
              (treatment) => treatment.id === choice.lightTreatment,
            );
            const dark = BACKDROP_TREATMENTS.find(
              (treatment) => treatment.id === choice.darkTreatment,
            );
            return (
              <div
                key={choice.variable}
                className="flex items-start gap-4 py-2.5"
              >
                <div
                  className="mt-0.5 h-9 w-16 shrink-0 rounded-md border border-primary"
                  style={{ background: `var(${choice.variable})` }}
                />
                <div className="flex min-w-0 flex-1 flex-col gap-1">
                  <code className="text-body text-primary">{choice.token}</code>
                  <p className="text-body-sm text-secondary">{choice.use}</p>
                  <p className="text-body-sm text-tertiary">
                    {light?.name} light / {dark?.name} dark
                  </p>
                </div>
              </div>
            );
          })}
        </div>
      </Section>

      <Section
        title="Layer 0 — translucency"
        description="Glass is the one ramp that is not a hue: each step is the mode\u2019s own surface colour at an increasing opacity, so a hover moves one step up rather than holding its own literal. It stays separate because genuine translucency \u2014 something behind showing through \u2014 is a different axis from colour, and it is the one place alpha is legitimately baked into a value. Components never reference these."
      >
        <div className="flex flex-col gap-8">
          {RAMPS.map((ramp) => (
            <div key={ramp.id} className="flex flex-col gap-3">
              <div className="flex flex-col gap-1">
                <h3 className="text-body text-primary">{ramp.name}</h3>
                <p className="max-w-2xl text-body-sm text-secondary">
                  {ramp.description}
                </p>
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

      <Section
        title="Layer 2 — the roles"
        description="Fifteen names, and each had to earn one. A role exists when light and dark take different ramp steps, so no single class is correct in both \u2014 or when the name enforces a rule a ramp cannot state, like there being exactly three levels of text. Everything else is written as a ramp step, because a name in front of a number hides the choice instead of recording it."
      >
        <div className="flex flex-col gap-8">
          {ROLE_GROUPS.map((group) => (
            <div key={group.id} className="flex flex-col gap-2">
              <div className="flex flex-col gap-1">
                <h3 className="text-body text-primary">{group.name}</h3>
                <p className="max-w-2xl text-body-sm text-secondary">
                  {group.description}
                </p>
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

      <Section
        title="Deliberate exceptions"
        description="Literal values exist only in the palette, and nothing above it holds one — except these. The list is short and complete on purpose: a vague exception policy is how a layered system quietly erodes."
      >
        {/* No swatch here to do the separating, so these entries keep a little
            structure — the token name leads and the spacing groups it with its
            reason. Still no card: this is prose, not data. */}
        <div className="flex flex-col gap-5">
          {EXCEPTIONS.map((exception) => (
            <div key={exception.name} className="flex flex-col gap-1">
              <code className="text-mono text-purple-12">{exception.name}</code>
              <p className="max-w-2xl text-body-sm text-secondary">
                {exception.why}
              </p>
            </div>
          ))}
        </div>
      </Section>

      <Note>
        Every dark value in this system is authored rather than observed — the
        design exploration it was derived from is light-only. Toggle the mode in
        the sidebar and treat anything that looks wrong as a finding, not a
        given.
      </Note>
    </>
  );
}
