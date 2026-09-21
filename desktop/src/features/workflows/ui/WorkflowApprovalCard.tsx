import type { WorkflowApproval } from "@/shared/api/types";
import { useTranslation } from "@/i18n";

type WorkflowApprovalCardProps = {
  approval: WorkflowApproval;
};

export function WorkflowApprovalCard({ approval }: WorkflowApprovalCardProps) {
  const { t } = useTranslation();
  const isExpired = new Date(approval.expiresAt) < new Date();

  if (approval.status !== "pending" || isExpired) {
    return null;
  }

  return (
    <div
      className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3"
      data-testid="workflow-approval-card"
    >
      <p className="mb-2 text-sm font-medium">
        {t("workflows.approval.title")}
      </p>

      <p className="mb-2 text-xs text-muted-foreground">
        {t("workflows.approval.approver", {
          approver: approval.approverSpec,
        })}
      </p>
      <p className="mb-2 text-xs text-muted-foreground">
        {t("workflows.approval.expires", {
          time: new Date(approval.expiresAt).toLocaleString(),
        })}
      </p>
      <p className="text-xs text-muted-foreground" role="status">
        {t("workflows.approval.unavailable")}
      </p>
    </div>
  );
}
