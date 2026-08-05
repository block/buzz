import { useQuery } from "@tanstack/react-query";
import { AlertTriangle, Eye, GitBranch, ShieldCheck } from "lucide-react";

import { readCausalLedger } from "@/shared/api/tauriCausalLedger";
import { Badge } from "@/shared/ui/badge";

import { inspectCausalCandidate } from "./causalCandidateInspection";

export function CausalCandidateCard({
  sessionId,
}: {
  sessionId: string | null;
}) {
  const query = useQuery({
    queryKey: ["causal-ledger", sessionId],
    queryFn: readCausalLedger,
    enabled: Boolean(sessionId),
    refetchInterval: 5_000,
  });
  const inspection = inspectCausalCandidate(query.data ?? [], sessionId);
  if (!inspection) return null;

  return (
    <section
      aria-label="Causal ledger candidate"
      className="mb-3 overflow-hidden rounded-xl border border-border/70 bg-background"
      data-testid="causal-candidate-card"
    >
      <div className="flex flex-wrap items-start justify-between gap-2 border-b border-border/60 p-4">
        <div>
          <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
            Durable candidate
          </p>
          <h3 className="mt-1 text-sm font-semibold">{inspection.task}</h3>
        </div>
        <Badge variant="outline" className="capitalize">
          {inspection.status}
        </Badge>
      </div>

      <CandidateList
        icon={Eye}
        items={inspection.capturedFacts}
        label="Captured facts"
      />
      <CandidateList
        empty="No causal claim has been made from this raw session."
        icon={GitBranch}
        items={inspection.inferredClaims}
        label="Inferred claims"
      />
      <CandidateList
        empty="No known evidence gaps."
        icon={AlertTriangle}
        items={inspection.missingCoverage}
        label="Missing coverage"
        warning
      />

      <div className="bg-primary/[0.04] p-4">
        <div className="flex items-start gap-2">
          <ShieldCheck
            aria-hidden
            className="mt-0.5 h-4 w-4 shrink-0 text-primary"
          />
          <div>
            <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
              Promotion gate
            </p>
            <p className="mt-1 text-sm font-medium">{inspection.nextGate}</p>
          </div>
        </div>
      </div>
    </section>
  );
}

function CandidateList({
  empty,
  icon: Icon,
  items,
  label,
  warning = false,
}: {
  empty?: string;
  icon: typeof Eye;
  items: readonly string[];
  label: string;
  warning?: boolean;
}) {
  return (
    <div className="border-b border-border/60 p-4">
      <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
        {label}
      </p>
      {items.length ? (
        <ul className="mt-2 space-y-2">
          {items.map((item) => (
            <li
              className={
                warning
                  ? "flex gap-2 text-xs text-amber-700 dark:text-amber-300"
                  : "flex gap-2 text-xs"
              }
              key={item}
            >
              <Icon aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span>{item}</span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="mt-2 text-xs text-muted-foreground">{empty}</p>
      )}
    </div>
  );
}
