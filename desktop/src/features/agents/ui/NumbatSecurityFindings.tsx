import { Octagon, ShieldAlert } from "lucide-react";

import type { NumbatFinding } from "@/shared/api/tauriNumbat";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";

export function NumbatSecurityFindings({
  activeTurnId,
  canCancelTurn,
  error,
  findings,
  onCancelTurn,
}: {
  activeTurnId: string | null;
  canCancelTurn: boolean;
  error: string | null;
  findings: NumbatFinding[];
  onCancelTurn: () => void;
}) {
  if (findings.length === 0 && !error) return null;

  return (
    <section
      aria-label="Guardian security findings"
      className="mb-3 space-y-2"
      data-testid="guardian-security-findings"
    >
      {findings
        .slice()
        .reverse()
        .map((finding) => {
          const canAct =
            canCancelTurn &&
            finding.turnId !== null &&
            finding.turnId === activeTurnId &&
            (finding.severity === "high" || finding.severity === "critical");
          return (
            <article
              className={cn(
                "rounded-lg border px-3 py-3",
                finding.severity === "critical"
                  ? "border-destructive/50 bg-destructive/10"
                  : finding.severity === "high"
                    ? "border-amber-500/40 bg-amber-500/10"
                    : "border-border/70 bg-muted/35",
              )}
              data-finding-id={finding.findingId}
              key={finding.findingId}
            >
              <div className="flex items-start gap-3">
                <ShieldAlert
                  aria-hidden="true"
                  className="mt-0.5 h-4 w-4 shrink-0 text-amber-600"
                />
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge variant="outline">
                      {finding.severity.toUpperCase()}
                    </Badge>
                    <span className="text-sm font-semibold">
                      {finding.title}
                    </span>
                  </div>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {finding.evidenceCount} correlated evidence event
                    {finding.evidenceCount === 1 ? "" : "s"}
                    {finding.sessionId
                      ? " · Session correlated"
                      : " · Local finding"}
                  </p>
                  <p className="mt-1 font-mono text-xs text-muted-foreground">
                    {finding.ruleId}
                  </p>
                </div>
                {canAct ? (
                  <Button
                    onClick={onCancelTurn}
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    <Octagon aria-hidden="true" className="h-3.5 w-3.5" />
                    Cancel turn
                  </Button>
                ) : null}
              </div>
            </article>
          );
        })}
      {error ? (
        <p className="text-xs text-muted-foreground">
          Guardian is temporarily unavailable: {error}
        </p>
      ) : null}
    </section>
  );
}
