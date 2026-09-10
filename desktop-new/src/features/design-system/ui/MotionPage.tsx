import { MOTION } from "@/shared/tokens/registry";
import { FoundationScale } from "./FoundationScale";
import { PageHeader, Section } from "./primitives";

export function MotionPage() {
  return (
    <>
      <PageHeader
        title="Motion"
        status="forming"
        intro="State changes are quick and geometry settles deliberately. Direct manipulation itself remains immediate: a panel follows the pointer with no smoothing, then uses the settling curve only after release."
      />

      <Section title="Roles">
        <FoundationScale
          items={MOTION.map((item) => ({
            token: item.token,
            value: item.value,
            use: item.use,
          }))}
        />
      </Section>

      <Section
        title="Rules"
        description="The values are small; the constraints are the system."
      >
        <ul className="flex list-disc flex-col gap-2 pl-5">
          {[
            "Direct manipulation follows the pointer exactly. Easing begins only after release.",
            "Never animate blur. Change opacity when glass enters or leaves.",
            "Reduced motion makes every transition immediate without changing state.",
            "Motion explains selection, entry, exit, or changed geometry; it never decorates still content.",
          ].map((rule) => (
            <li key={rule} className="text-body text-secondary">
              {rule}
            </li>
          ))}
        </ul>
      </Section>
    </>
  );
}
