import { useQueryClient } from "@tanstack/react-query";
import { CheckCircle2 } from "lucide-react";
import * as React from "react";

import { MeetingError } from "@/features/meetings/api";
import type { SubscribeIntent } from "@/features/meetings/api";
import {
  PAYMENT_STATUS_MAX_POLL_FAILURES,
  usePlansQuery,
  useSubscribeMutation,
  usePaymentStatusQuery,
} from "@/features/meetings/hooks";
import { InvoicePanel } from "@/features/meetings/ui/InvoicePanel";
import { PlanCard } from "@/features/meetings/ui/PlanCard";
import {
  stepFromPaymentStatus,
  stepFromSubscribeError,
  type SubscribeStep,
} from "@/features/meetings/ui/subscribeState";
import { Button } from "@/shared/ui/button";

type SubscribeViewProps = {
  /** Existing invoice to resume (from a `409 pending_invoice`), if any. */
  initialIntent?: SubscribeIntent;
  /** Fired once payment settles — the caller retries the flow that hit 402. */
  onSettled: () => void;
  /** Fired when the user dismisses the settled/close state. */
  onClose: () => void;
};

export function SubscribeView({
  initialIntent,
  onSettled,
  onClose,
}: SubscribeViewProps) {
  const queryClient = useQueryClient();
  const plansQuery = usePlansQuery();
  const subscribeMutation = useSubscribeMutation();

  const [step, setStep] = React.useState<SubscribeStep>(
    initialIntent
      ? { kind: "invoice", intent: initialIntent }
      : { kind: "plans" },
  );
  const [selectedPlan, setSelectedPlan] = React.useState<string | null>(
    initialIntent?.plan ?? null,
  );

  const invoiceIntentId =
    step.kind === "invoice" ? step.intent.intent_id : undefined;
  const paymentStatusQuery = usePaymentStatusQuery(
    invoiceIntentId,
    step.kind === "invoice",
  );

  // The poll gave up (persistent non-429 failure). A user who paid in that
  // window would otherwise sit on "updates automatically" forever — surface it
  // and offer a manual re-check (a window-focus return also retries).
  const pollStalled =
    paymentStatusQuery.isError &&
    !(
      paymentStatusQuery.error instanceof MeetingError &&
      paymentStatusQuery.error.status === 429
    ) &&
    paymentStatusQuery.failureCount >= PAYMENT_STATUS_MAX_POLL_FAILURES;

  const onSettledRef = React.useRef(onSettled);
  onSettledRef.current = onSettled;

  React.useEffect(() => {
    if (step.kind !== "invoice" || !paymentStatusQuery.data) return;
    const next = stepFromPaymentStatus(
      paymentStatusQuery.data,
      selectedPlan ?? step.intent.plan,
    );
    if (!next) return;
    setStep(next);
    if (next.kind === "settled") {
      // Refresh subscription/room state, but never the active LiveKit token
      // query — reminting the JWT mid-call bounces a host who is already in a
      // room (same class as the moderation-invalidation fix).
      void queryClient.invalidateQueries({
        predicate: (query) => {
          const key = query.queryKey;
          return (
            Array.isArray(key) && key[0] === "meetings" && key[2] !== "token"
          );
        },
      });
      onSettledRef.current();
    }
  }, [paymentStatusQuery.data, step, selectedPlan, queryClient]);

  const startSubscribe = (plan: string) => {
    setSelectedPlan(plan);
    subscribeMutation.mutate(plan, {
      onSuccess: (intent) => setStep({ kind: "invoice", intent }),
      onError: (error) => {
        const recovered = stepFromSubscribeError(error, plan);
        if (recovered) setStep(recovered);
      },
    });
  };

  if (step.kind === "settled") {
    return (
      <div
        className="flex flex-col items-center gap-3 py-6 text-center"
        data-testid="meeting-subscribe-settled"
      >
        <CheckCircle2 className="h-10 w-10 text-emerald-500" />
        <p className="text-sm font-medium">Subscription active</p>
        <p className="text-xs text-muted-foreground">
          Payment confirmed. You can host meetings now.
        </p>
        <Button onClick={onClose} size="sm" type="button">
          Done
        </Button>
      </div>
    );
  }

  if (step.kind === "invoice") {
    return (
      <div className="space-y-3">
        <InvoicePanel
          intent={step.intent}
          onRegenerate={() => startSubscribe(selectedPlan ?? step.intent.plan)}
          regenerating={subscribeMutation.isPending}
          pollStalled={pollStalled}
          onRecheck={() => void paymentStatusQuery.refetch()}
          rechecking={paymentStatusQuery.isFetching}
        />
        <Button
          onClick={() => setStep({ kind: "plans" })}
          size="sm"
          type="button"
          variant="ghost"
        >
          Choose a different plan
        </Button>
      </div>
    );
  }

  if (step.kind === "expired") {
    return (
      <div
        className="space-y-3 text-center"
        data-testid="meeting-subscribe-expired"
      >
        <p className="text-sm font-medium">Payment didn't complete</p>
        <p className="text-xs text-muted-foreground">
          The invoice expired or the payment failed. Try again.
        </p>
        <Button
          disabled={subscribeMutation.isPending}
          onClick={() => startSubscribe(step.plan)}
          size="sm"
          type="button"
        >
          {subscribeMutation.isPending ? "Creating…" : "Get a new invoice"}
        </Button>
      </div>
    );
  }

  // step.kind === "plans"
  const subscribeErrorMessage =
    subscribeMutation.error instanceof MeetingError &&
    subscribeMutation.error.kind !== "pending_invoice"
      ? subscribeMutation.error.message
      : subscribeMutation.error
        ? "Couldn't start the subscription. Try again."
        : undefined;

  return (
    <div className="space-y-3" data-testid="meeting-subscribe-plans">
      <p className="text-sm text-muted-foreground">
        Hosting a meeting needs an active HiveTalk subscription. Pick a plan and
        pay the Lightning invoice with any wallet.
      </p>

      {plansQuery.isLoading ? (
        <p className="rounded-xl border border-dashed border-border/70 px-4 py-6 text-center text-sm text-muted-foreground">
          Loading plans…
        </p>
      ) : plansQuery.isError ? (
        <div className="space-y-2 rounded-xl border border-border/70 p-4 text-center">
          <p className="text-sm">Couldn't load subscription plans.</p>
          <Button
            onClick={() => void plansQuery.refetch()}
            size="sm"
            type="button"
            variant="outline"
          >
            Retry
          </Button>
        </div>
      ) : (plansQuery.data?.length ?? 0) === 0 ? (
        <p className="rounded-xl border border-dashed border-border/70 px-4 py-6 text-center text-sm text-muted-foreground">
          This relay's provider isn't offering any plans right now.
        </p>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2">
          {plansQuery.data?.map((plan) => (
            <PlanCard
              disabled={subscribeMutation.isPending}
              key={plan.plan}
              onSelect={startSubscribe}
              pending={
                subscribeMutation.isPending && selectedPlan === plan.plan
              }
              plan={plan}
              selected={selectedPlan === plan.plan}
            />
          ))}
        </div>
      )}

      {subscribeErrorMessage ? (
        <p className="text-xs text-destructive">{subscribeErrorMessage}</p>
      ) : null}
    </div>
  );
}
