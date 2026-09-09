import { GROWTH_PROCEDURE } from "@/shared/tokens/registry";

import { PageHeader, Section } from "./primitives";

const AUDIT: Array<[string, string]> = [
  [
    "A newly introduced vocabulary word",
    "Reported on its own line — this changes the system's shape",
  ],
  ["Two names with identical light and dark values", "Merged into one"],
  [
    "Values within a hair of each other",
    "Reported for a person to judge — usually drift, occasionally deliberate",
  ],
  ["Names nothing references", "Removed"],
  [
    "Names that do not parse under the grammar",
    "Reported with the correct spelling",
  ],
  [
    "A role value that is a literal rather than a ramp reference",
    "Reported, unless on the deliberate-exception list",
  ],
  [
    "A proposed role used in several places by more than one feature",
    "Nominated for promotion",
  ],
  [
    "A proposed role still used once, some time later",
    "Nominated for deletion",
  ],
  ["A deprecated role still referenced", "Fails, naming its replacement"],
  [
    "Any legal text-on-background pairing below contrast threshold",
    "Fails, in either mode",
  ],
];

export function GrowthPage() {
  return (
    <>
      <PageHeader title="Growing the system" />

      <Section title="The procedure">
        <ol className="flex flex-col gap-2 rounded-xl bg-inset px-5 py-4">
          {GROWTH_PROCEDURE.map((step, index) => (
            <li key={step} className="flex gap-3">
              <span className="w-4 shrink-0 text-label text-accent">
                {index + 1}
              </span>
              <span className="text-body text-secondary">{step}</span>
            </li>
          ))}
        </ol>
      </Section>

      <Section title="The audit">
        <div className="rounded-xl bg-inset px-5">
          {AUDIT.map(([check, action]) => (
            <div
              key={check}
              className="flex flex-col gap-1 border-b border-tertiary py-3 last:border-b-0 sm:flex-row sm:gap-4"
            >
              <span className="flex-1 text-caption text-primary">{check}</span>
              <span className="flex-1 text-caption text-secondary">
                {action}
              </span>
            </div>
          ))}
        </div>
      </Section>
    </>
  );
}
