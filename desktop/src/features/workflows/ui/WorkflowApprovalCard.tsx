import { Check, X } from "lucide-react";
import * as React from "react";

import { useApprovalMutation } from "@/features/workflows/hooks";
import type { WorkflowApproval } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Textarea } from "@/shared/ui/textarea";

type WorkflowApprovalCardProps = {
  approval: WorkflowApproval;
};

export function WorkflowApprovalCard({ approval }: WorkflowApprovalCardProps) {
  const [note, setNote] = React.useState("");
  const approvalMutation = useApprovalMutation();

  const isExpired = new Date(approval.expiresAt) < new Date();

  if (approval.status !== "pending" || isExpired) {
    return null;
  }

  return (
    <div
      className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3"
      data-testid="workflow-approval-card"
    >
      <p className="mb-2 text-sm font-medium">Approval Required</p>

      {/*
        The exact package being approved. Approving from a card that shows only
        approver and expiry is approving blind, which for a gate that authorises
        an external send defeats the point of the gate. Read-only and
        pre-wrapped so the reviewed text is byte-for-byte what was submitted.
      */}
      {approval.requestMessage ? (
        <pre
          className="mb-2 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md border bg-background/60 p-2 text-xs font-mono"
          data-testid="workflow-approval-request-message"
        >
          {approval.requestMessage}
        </pre>
      ) : (
        <p
          className="mb-2 rounded-md border border-red-500/40 bg-red-500/10 p-2 text-xs text-red-700"
          data-testid="workflow-approval-request-missing"
        >
          No request package was recorded for this gate, so there is nothing to
          review. Approval is disabled. Deny it and re-run the workflow.
        </p>
      )}

      <p className="mb-2 text-xs text-muted-foreground">
        Approver: {approval.approverSpec}
      </p>
      <p className="mb-2 text-xs text-muted-foreground">
        Expires: {new Date(approval.expiresAt).toLocaleString()}
      </p>

      <Textarea
        aria-label="Approval note"
        className="mb-2 h-16 resize-none text-xs"
        onChange={(event) => setNote(event.target.value)}
        placeholder="Optional note..."
        value={note}
      />

      <div className="flex gap-2">
        <Button
          className="flex-1 bg-green-600 text-white hover:bg-green-700"
          // Deny stays available with no package; approve does not. You cannot
          // consent to something you were never shown.
          disabled={approvalMutation.isPending || !approval.requestMessage}
          onClick={() =>
            approvalMutation.mutate({
              token: approval.token,
              action: "grant",
              note: note || undefined,
            })
          }
          size="sm"
        >
          <Check className="mr-1 h-4 w-4" />
          Approve
        </Button>
        <Button
          className="flex-1"
          disabled={approvalMutation.isPending}
          onClick={() =>
            approvalMutation.mutate({
              token: approval.token,
              action: "deny",
              note: note || undefined,
            })
          }
          size="sm"
          variant="destructive"
        >
          <X className="mr-1 h-4 w-4" />
          Deny
        </Button>
      </div>
    </div>
  );
}
