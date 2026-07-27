import type {
  BriefSection,
  CitedFinding,
} from "@/features/command-console/domain/briefContracts";
import { cn } from "@/shared/lib/cn";
import { Card, CardContent, CardHeader } from "@/shared/ui/card";

import { SECTION_LABELS } from "./briefPresentation";
import { SourceCitationLink } from "./SourceCitationLink";

export function BriefSectionCard({
  findings,
  prominent = false,
  section,
}: {
  findings: readonly CitedFinding[];
  prominent?: boolean;
  section: BriefSection;
}) {
  return (
    <Card
      className={cn(
        "h-full overflow-hidden",
        prominent &&
          "border-[#d8aa4f]/50 bg-[#d8aa4f]/8 shadow-[0_0_0_1px_rgba(216,170,79,0.08)]",
      )}
      data-testid={`brief-section-${section}`}
    >
      <CardHeader className="pb-3">
        <h3
          className={cn(
            "text-base font-semibold",
            prominent && "text-[#d8aa4f]",
          )}
        >
          {SECTION_LABELS[section]}
        </h3>
      </CardHeader>
      <CardContent>
        {findings.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No supported finding was available for this section.
          </p>
        ) : (
          <ul className="space-y-3 text-sm leading-relaxed">
            {findings.map((finding) => (
              <li
                className="border-b border-border/50 pb-3 last:border-0 last:pb-0"
                key={`${finding.text}-${finding.sourceIds.join("-")}`}
              >
                <span>{finding.text}</span>{" "}
                {finding.sourceIds.map((sourceId) => (
                  <SourceCitationLink key={sourceId} sourceId={sourceId} />
                ))}
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}
