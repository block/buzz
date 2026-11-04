// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import type { WorkflowApproval } from "@/shared/api/types";

type WorkflowApprovalCardProps = {
  approval: WorkflowApproval;
};

export function WorkflowApprovalCard({ approval }: WorkflowApprovalCardProps) {
  const isExpired = new Date(approval.expiresAt) < new Date();

  if (approval.status !== "pending" || isExpired) {
    return null;
  }

  return (
    <div
      className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3"
      data-testid="workflow-approval-card"
    >
      <p className="mb-2 text-sm font-medium">{uiT("Approval Required")}</p>
      <p className="mb-2 text-xs text-muted-foreground">
        {uiT("Approver: {0}", { p0: approval.approverSpec })}
      </p>
      <p className="mb-2 text-xs text-muted-foreground">
        {uiT("Expires: {0}", {
          p0: new Date(approval.expiresAt).toLocaleString(),
        })}
      </p>
      <p className="text-xs text-muted-foreground" role="status">
        {uiT("Approval actions are not yet available in Desktop.")}
      </p>
    </div>
  );
}
