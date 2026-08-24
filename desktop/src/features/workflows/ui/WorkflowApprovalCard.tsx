import { useApprovalMutation } from "@/features/workflows/hooks";
import type { WorkflowApproval } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";

type WorkflowApprovalCardProps = {
  approval: WorkflowApproval;
};

export function WorkflowApprovalCard({ approval }: WorkflowApprovalCardProps) {
  const isExpired = new Date(approval.expiresAt) < new Date();
  const approvalMutation = useApprovalMutation();

  if (approval.status !== "pending" || isExpired) {
    return null;
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
      {approvalMutation.isError ? (
        <p className="mb-2 text-xs text-red-500" role="status">
          {approvalMutation.error?.message}
        </p>
      ) : null}
      <div className="flex gap-2">
        <Button
          size="sm"
          data-testid="approve-approval"
          disabled={approvalMutation.isPending}
          onClick={() =>
            approvalMutation.mutate({
              token: approval.approvalRef,
              action: "grant",
            })
          }
        >
          Approve
        </Button>
        <Button
          size="sm"
          variant="outline"
          data-testid="deny-approval"
          disabled={approvalMutation.isPending}
          onClick={() =>
            approvalMutation.mutate({
              token: approval.approvalRef,
              action: "deny",
            })
          }
        >
          Deny
        </Button>
      </div>
    </div>
  );
}
