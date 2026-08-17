import { useApprovalMutation } from "@/features/workflows/hooks";
import type { WorkflowApproval } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";

type WorkflowApprovalCardProps = {
  approval: WorkflowApproval;
};

export function WorkflowApprovalCard({ approval }: WorkflowApprovalCardProps) {
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
      <p className="mb-2 text-xs text-muted-foreground">
        Approver: {approval.approverSpec}
      </p>
      <p className="mb-2 text-xs text-muted-foreground">
        Expires: {new Date(approval.expiresAt).toLocaleString()}
      </p>
      <div className="flex gap-2">
        <Button
          disabled={approvalMutation.isPending}
          onClick={() => {
            void approvalMutation.mutateAsync({
              action: "grant",
              token: approval.approvalRef,
            });
          }}
          size="sm"
          type="button"
        >
          Approve
        </Button>
        <Button
          disabled={approvalMutation.isPending}
          onClick={() => {
            void approvalMutation.mutateAsync({
              action: "deny",
              token: approval.approvalRef,
            });
          }}
          size="sm"
          type="button"
          variant="outline"
        >
          Deny
        </Button>
      </div>
    </div>
  );
}
