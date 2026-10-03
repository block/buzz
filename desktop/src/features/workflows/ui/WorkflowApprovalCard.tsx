import * as React from "react";

import { useApprovalMutation } from "@/features/workflows/hooks";
import type { WorkflowApproval } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";

type WorkflowApprovalCardProps = {
  approval: WorkflowApproval;
};

export function WorkflowApprovalCard({ approval }: WorkflowApprovalCardProps) {
  const mutation = useApprovalMutation();
  const [submitted, setSubmitted] = React.useState<{
    approvalRef: string;
    action: "grant" | "deny";
  } | null>(null);
  const expiresAt = Date.parse(approval.expiresAt);
  const isExpired = !Number.isFinite(expiresAt) || expiresAt <= Date.now();
  const submittedAction =
    submitted?.approvalRef === approval.approvalRef ? submitted.action : null;
  const isSubmitting = mutation.isPending || submittedAction !== null;

  if (approval.status !== "pending") {
    return null;
  }

  async function act(action: "grant" | "deny") {
    if (isExpired || isSubmitting) return;
    setSubmitted({ approvalRef: approval.approvalRef, action });
    try {
      await mutation.mutateAsync({
        action,
        token: approval.approvalRef,
      });
    } catch {
      setSubmitted((current) =>
        current?.approvalRef === approval.approvalRef ? null : current,
      );
    }
  }

  return (
    <div
      className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3"
      data-testid="workflow-approval-card"
    >
      <p className="mb-2 text-sm font-medium">Approval Required</p>
      <p className="mb-2 text-xs text-muted-foreground">
        Approver: {approval.approverSpec}
      </p>
      <p className="mb-2 text-xs text-muted-foreground">
        Expires: {new Date(approval.expiresAt).toLocaleString()}
      </p>
      {isExpired ? (
        <p className="text-xs text-muted-foreground" role="status">
          This approval request has expired.
        </p>
      ) : (
        <div className="flex flex-wrap gap-2">
          <Button
            data-testid="workflow-approval-grant"
            disabled={isSubmitting}
            onClick={() => void act("grant")}
            size="sm"
            type="button"
          >
            {submittedAction === "grant" ? "Approving..." : "Approve"}
          </Button>
          <Button
            data-testid="workflow-approval-deny"
            disabled={isSubmitting}
            onClick={() => void act("deny")}
            size="sm"
            type="button"
            variant="outline"
          >
            {submittedAction === "deny" ? "Denying..." : "Deny"}
          </Button>
        </div>
      )}
      {mutation.error instanceof Error ? (
        <p className="mt-2 text-xs text-red-400" role="alert">
          {mutation.error.message}
        </p>
      ) : null}
    </div>
  );
}
