import { MessageSquare, Mic, RefreshCw } from "lucide-react";
import * as React from "react";

import type { BriefDecision } from "@/features/command-console/domain/briefDecisions";
import type {
  DecisionDirectionSource,
  DecisionExecution,
  DecisionExecutionStatus,
} from "@/features/command-console/domain/decisionExecutionStore";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/shared/ui/card";
import { Textarea } from "@/shared/ui/textarea";

const STATUS_LABELS: Record<DecisionExecutionStatus, string> = {
  queued: "Queued",
  in_progress: "In progress",
  blocked: "Blocked",
  completed: "Complete",
  failed: "Failed",
  stalled: "Stalled",
};

function statusVariant(status: DecisionExecutionStatus) {
  if (status === "completed") return "success" as const;
  if (status === "blocked" || status === "stalled") return "warning" as const;
  if (status === "failed") return "destructive" as const;
  return "secondary" as const;
}

export type BriefDecisionActions = Readonly<{
  executions: readonly DecisionExecution[];
  pendingKeys: ReadonlySet<string>;
  issue: (
    decision: BriefDecision,
    direction: string,
    source: DecisionDirectionSource,
  ) => Promise<void> | void;
  retry: (
    decision: BriefDecision,
    execution: DecisionExecution,
  ) => Promise<void> | void;
  openConversation: (execution: DecisionExecution) => Promise<void> | void;
}>;

const INERT_ACTIONS: BriefDecisionActions = {
  executions: [],
  pendingKeys: new Set(),
  issue: () => {},
  retry: () => {},
  openConversation: () => {},
};

function executionForDecision(
  decision: BriefDecision,
  executions: readonly DecisionExecution[],
) {
  return (
    executions.find((execution) => execution.key === decision.key) ??
    executions.find(
      (execution) =>
        execution.actionId === decision.actionId &&
        execution.status === "completed",
    )
  );
}

function DecisionCard({
  actions,
  decision,
}: {
  actions: BriefDecisionActions;
  decision: BriefDecision;
}) {
  const [direction, setDirection] = React.useState("");
  const execution = executionForDecision(decision, actions.executions);
  const pending = actions.pendingKeys.has(decision.key);
  const retryable =
    execution?.status === "failed" || execution?.status === "stalled";

  if (execution) {
    return (
      <div className="rounded-xl border border-border/70 bg-background/40 p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="font-medium">{decision.coaA}</p>
          <Badge variant={statusVariant(execution.status)}>
            {STATUS_LABELS[execution.status]}
          </Badge>
        </div>
        <p className="mt-3 text-sm text-muted-foreground">
          <span className="font-medium text-foreground">CO direction:</span>{" "}
          {execution.direction}
        </p>
        <p className="mt-1 text-sm text-muted-foreground">
          {execution.statusText ?? "Direction sent to the Chief of Staff."}
        </p>
        <div className="mt-4 flex flex-wrap gap-2">
          {retryable ? (
            <Button
              disabled={pending}
              onClick={() => actions.retry(decision, execution)}
              size="sm"
              type="button"
              variant="outline"
            >
              <RefreshCw aria-hidden="true" />
              Retry
            </Button>
          ) : null}
          {execution.channelId ? (
            <Button
              onClick={() => actions.openConversation(execution)}
              size="sm"
              type="button"
              variant="outline"
            >
              <MessageSquare aria-hidden="true" />
              Open Chief of Staff
            </Button>
          ) : null}
        </div>
      </div>
    );
  }

  return (
    <div className="rounded-xl border border-border/70 bg-background/40 p-4">
      <div className="grid gap-3 lg:grid-cols-2">
        <div className="rounded-lg border border-primary/40 bg-primary/10 p-3">
          <p className="text-xs font-semibold uppercase tracking-wide text-primary">
            COA A — Recommended
          </p>
          <p className="mt-2 text-sm leading-relaxed">{decision.coaA}</p>
          <Button
            className="mt-3"
            disabled={pending}
            onClick={() => actions.issue(decision, decision.coaA, "coa_a")}
            size="sm"
            type="button"
          >
            Direct COA A
          </Button>
        </div>

        {decision.coaB ? (
          <div className="rounded-lg border border-border/70 p-3">
            <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              COA B — Alternative
            </p>
            <p className="mt-2 text-sm leading-relaxed">{decision.coaB}</p>
            <Button
              className="mt-3"
              disabled={pending}
              onClick={() =>
                actions.issue(decision, decision.coaB ?? "", "coa_b")
              }
              size="sm"
              type="button"
              variant="outline"
            >
              Direct COA B
            </Button>
          </div>
        ) : null}
      </div>

      <div className="mt-4">
        <label
          className="text-sm font-medium"
          htmlFor={`command-direction-${decision.actionId}`}
        >
          Your direction
        </label>
        <Textarea
          autoCapitalize="sentences"
          autoCorrect="on"
          className="mt-2"
          id={`command-direction-${decision.actionId}`}
          onChange={(event) => setDirection(event.target.value)}
          placeholder="Type or dictate your direction…"
          spellCheck
          value={direction}
        />
        <div className="mt-2 flex flex-wrap items-center justify-between gap-3">
          <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <Mic aria-hidden="true" className="h-3.5 w-3.5" />
            Use your keyboard microphone or macOS Dictation.
          </p>
          <Button
            disabled={pending || direction.trim().length === 0}
            onClick={() => actions.issue(decision, direction.trim(), "user")}
            size="sm"
            type="button"
          >
            Issue direction
          </Button>
        </div>
      </div>
    </div>
  );
}

export function BriefDecisionSection({
  actions = INERT_ACTIONS,
  decisions,
}: {
  actions?: BriefDecisionActions;
  decisions: readonly BriefDecision[];
}) {
  return (
    <Card
      className="overflow-hidden border-[#d8aa4f]/50 bg-[#d8aa4f]/8 shadow-[0_0_0_1px_rgba(216,170,79,0.08)]"
      data-testid="brief-section-decisions"
    >
      <CardHeader className="pb-3">
        <CardTitle className="text-base text-[#d8aa4f]">
          Decisions and approvals required
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        {decisions.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No command decision is required.
          </p>
        ) : (
          decisions.map((decision) => (
            <DecisionCard
              actions={actions}
              decision={decision}
              key={decision.key}
            />
          ))
        )}
      </CardContent>
    </Card>
  );
}
