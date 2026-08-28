import { useSubscriptionQuery } from "@/features/meetings/hooks";
import { subscriptionBadgeModel } from "@/features/meetings/ui/subscribeState";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";

const TONE_VARIANT = {
  active: "success",
  warning: "warning",
  inactive: "outline",
} as const;

type SubscriptionStatusBadgeProps = {
  /** Open the subscribe dialog (renew / manage). */
  onManage: () => void;
};

export function SubscriptionStatusBadge({
  onManage,
}: SubscriptionStatusBadgeProps) {
  const { data, isLoading } = useSubscriptionQuery();
  if (isLoading) return null;

  const model = subscriptionBadgeModel(data, Date.now());
  if (!model) return null;

  return (
    <div
      className="flex items-center gap-2"
      data-testid="meeting-subscription-badge"
    >
      <div className="flex flex-col items-end">
        <Badge variant={TONE_VARIANT[model.tone]}>{model.label}</Badge>
        {model.expiryText ? (
          <span className="text-2xs text-muted-foreground">
            {model.expiryText}
          </span>
        ) : null}
      </div>
      {model.showRenew ? (
        <Button onClick={onManage} size="xs" type="button" variant="outline">
          Renew
        </Button>
      ) : null}
    </div>
  );
}
