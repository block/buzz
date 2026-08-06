import * as React from "react";
import { ShieldCheck } from "lucide-react";
import { toast } from "sonner";

import {
  readShepherdEvidence,
  settleShepherdRun,
  type StoredShepherdEvidence,
} from "@/shared/api/tauriShepherd";
import { Button } from "@/shared/ui/button";

type Props = {
  agentPubkey: string;
  channelId: string;
  sessionId: string;
  refreshKey?: number;
};

export function ShepherdEvidencePanel({
  agentPubkey,
  channelId,
  sessionId,
  refreshKey = 0,
}: Props) {
  const [records, setRecords] = React.useState<StoredShepherdEvidence[]>([]);

  React.useEffect(() => {
    let active = true;
    void readShepherdEvidence(agentPubkey, channelId, sessionId)
      .then((value) => active && setRecords(value))
      .catch(() => active && setRecords([]));
    return () => {
      active = false;
    };
  }, [agentPubkey, channelId, sessionId, refreshKey]);

  if (records.length === 0) return null;

  async function settle(
    record: StoredShepherdEvidence,
    action: "select" | "apply" | "discard",
  ) {
    const workspacePath = window.prompt(
      `Enter the local Shepherd workspace for ${record.sourceRunRef}:`,
    );
    if (!workspacePath) return;
    if (
      (action === "apply" || action === "discard") &&
      !window.confirm(
        `${action === "apply" ? "Apply" : "Discard"} Shepherd run ${record.sourceRunRef}?`,
      )
    )
      return;
    try {
      const result = await settleShepherdRun(
        workspacePath,
        record.sourceRunRef,
        action,
      );
      toast.success(result.message || `Shepherd run ${action} completed.`);
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : `Shepherd ${action} failed.`,
      );
    }
  }

  return (
    <section
      className="mb-2 rounded-lg border bg-muted/30 p-3"
      data-testid="shepherd-evidence-panel"
    >
      <div className="flex items-center gap-2 text-sm font-medium">
        <ShieldCheck className="h-4 w-4" /> Shepherd evidence
      </div>
      <p className="mt-1 text-xs text-muted-foreground">
        Boundary effects only. Raw prompts, tool output, and file contents are
        not retained.
      </p>
      {records.map((record) => (
        <div className="mt-3 border-t pt-3" key={record.sourceRunRef}>
          <p className="truncate text-xs font-medium" title={record.sourceRunRef}>
            Run {record.sourceRunRef} · {record.evidence.totalEffects} effects
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            {record.evidence.effectTypes.join(", ") || "No typed effects"}
          </p>
          <div className="mt-2 flex flex-wrap gap-2">
            {(["select", "apply", "discard"] as const).map((action) => (
              <Button
                key={action}
                size="sm"
                type="button"
                variant={action === "apply" ? "default" : "outline"}
                onClick={() => void settle(record, action)}
              >
                {action[0].toUpperCase() + action.slice(1)}
              </Button>
            ))}
          </div>
        </div>
      ))}
    </section>
  );
}
