import { ShieldAlert } from "lucide-react";

import type { NumbatFinding } from "@/shared/api/tauriNumbat";
import { Badge } from "@/shared/ui/badge";
import { cn } from "@/shared/lib/cn";

export function NumbatSecurityFindings({
  error,
  findings,
  health,
}: {
  error: string | null;
  findings: NumbatFinding[];
  health: {
    state: "configured" | "disconnected" | "unsupported" | "stale";
    detail: string;
  } | null;
}) {
  if (findings.length === 0 && !error && !health) return null;

  return (
    <section
      aria-label="Guardian security findings"
      className="mb-3 space-y-2"
      data-testid="guardian-security-findings"
    >
      {health ? (
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Badge variant="outline">{health.state}</Badge>
          <span>{health.detail}</span>
        </div>
      ) : null}
      {findings
        .slice()
        .reverse()
        .map((finding) => {
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
