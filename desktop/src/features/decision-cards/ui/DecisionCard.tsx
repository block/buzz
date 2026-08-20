import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowUpRight,
  Check,
  CircleAlert,
  FilePenLine,
  ShieldCheck,
  X,
} from "lucide-react";
import * as React from "react";

import {
  type DecisionCardChoice,
  parseDecisionCard,
  parseDecisionResponse,
  publishDecisionResponse,
  selectDecisionResponse,
} from "@/features/decision-cards/lib/decisionCards";
import type { TimelineMessage } from "@/features/messages/types";
import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_STREAM_DECISION_RESPONSE } from "@/shared/constants/kinds";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Textarea } from "@/shared/ui/textarea";

const choicePresentation = {
  approve: {
    label: "Approve",
    outcome: "Approved",
    icon: Check,
    variant: "default",
  },
  redraft: {
    label: "Redraft",
    outcome: "Redraft requested",
    icon: FilePenLine,
    variant: "outline",
  },
  escalate: {
    label: "Escalate",
    outcome: "Escalated",
    icon: ArrowUpRight,
    variant: "secondary",
  },
  reject: {
    label: "Reject",
    outcome: "Rejected",
    icon: X,
    variant: "destructive",
  },
} as const;

function responseQueryKey(cardEventId: string) {
  return ["decision-card-response", cardEventId] as const;
}

export function DecisionCard({
  channelId,
  message,
}: {
  channelId: string | null;
  message: TimelineMessage;
}) {
  const parsed = React.useMemo(
    () => parseDecisionCard(message.tags ?? []),
    [message.tags],
  );
  const queryClient = useQueryClient();
  const [note, setNote] = React.useState("");
  const responseQuery = useQuery({
    enabled: parsed !== null,
    queryKey: responseQueryKey(message.id),
    queryFn: () =>
      relayClient.fetchEvents({
        kinds: [KIND_STREAM_DECISION_RESPONSE],
        limit: 20,
        "#e": [message.id],
      }),
    staleTime: 10_000,
  });
  const responseEvent = parsed
    ? selectDecisionResponse(
        responseQuery.data ?? [],
        parsed.payload.card_id,
        parsed.payloadHash,
      )
    : null;
  const response = responseEvent
    ? parseDecisionResponse(responseEvent.tags)
    : null;
  const responseMutation = useMutation({
    mutationFn: (decision: DecisionCardChoice) => {
      if (!parsed || !channelId) {
        throw new Error("Decision card is missing its channel context.");
      }
      return publishDecisionResponse({
        cardEventId: message.id,
        cardId: parsed.payload.card_id,
        channelId,
        decision,
        note,
        payloadHash: parsed.payloadHash,
        rootEventId: message.rootId,
      });
    },
    onSuccess: (event) => {
      queryClient.setQueryData<RelayEvent[]>(responseQueryKey(message.id), [
        event,
      ]);
    },
  });

  if (!parsed) {
    return <p className="text-sm text-destructive">Invalid decision card.</p>;
  }

  const { payload } = parsed;
  const expired =
    payload.expires_at !== undefined &&
    payload.expires_at <= Math.floor(Date.now() / 1_000);

  return (
    <section
      className="mt-2 overflow-hidden rounded-2xl border border-border/70 bg-card shadow-sm"
      data-testid="decision-card"
    >
      <div className="border-b border-border/60 bg-gradient-to-br from-primary/10 via-card to-card p-4">
        <div className="mb-3 flex flex-wrap items-center gap-2">
          <Badge variant="warning">Decision</Badge>
          {payload.shadow ? <Badge variant="info">Shadow</Badge> : null}
          <span className="ml-auto font-mono text-2xs text-muted-foreground">
            {payload.card_id.slice(0, 8)}
          </span>
        </div>
        <h3 className="text-base font-semibold tracking-tight">
          {payload.title}
        </h3>
        <p className="mt-1 text-sm text-muted-foreground">
          {payload.situation}
        </p>
      </div>

      <div className="grid gap-3 p-4 sm:grid-cols-2">
        <div className="rounded-xl bg-emerald-500/8 p-3">
          <div className="mb-1 flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-emerald-700 dark:text-emerald-300">
            <ShieldCheck className="size-4" /> Recommendation
          </div>
          <p className="text-sm">{payload.recommendation}</p>
        </div>
        <div className="rounded-xl bg-amber-500/8 p-3">
          <div className="mb-1 flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-amber-700 dark:text-amber-300">
            <CircleAlert className="size-4" /> Risk
          </div>
          <p className="text-sm">{payload.risk}</p>
        </div>
      </div>

      <div className="px-4 pb-4">
        <p className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Exact proposed action
        </p>
        <p className="rounded-xl border border-border/60 bg-muted/30 p-3 text-sm">
          {payload.proposed_action}
        </p>
        {payload.record_url ? (
          <a
            className="mt-2 inline-flex items-center gap-1 text-xs font-medium text-primary hover:underline"
            href={payload.record_url}
            rel="noreferrer"
            target="_blank"
          >
            Open authoritative record <ArrowUpRight className="size-3" />
          </a>
        ) : null}
      </div>

      <div className="border-t border-border/60 bg-muted/20 p-4">
        {response ? (
          <DecisionRecordedState response={response} />
        ) : expired ? (
          <p className="text-sm font-medium text-muted-foreground">
            This decision card has expired.
          </p>
        ) : responseQuery.isError ? (
          <p className="text-sm font-medium text-destructive">
            Prior decisions could not be verified. Actions are disabled.
          </p>
        ) : (
          <>
            <Textarea
              aria-label="Decision note"
              className="mb-3 min-h-16 resize-none"
              disabled={responseQuery.isPending || responseMutation.isPending}
              onChange={(event) => setNote(event.target.value)}
              placeholder="Optional note for the durable receipt…"
              value={note}
            />
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
              {payload.choices.map((choice) => {
                const presentation = choicePresentation[choice];
                const Icon = presentation.icon;
                return (
                  <Button
                    disabled={
                      responseQuery.isPending ||
                      responseMutation.isPending ||
                      !channelId
                    }
                    key={choice}
                    onClick={() => responseMutation.mutate(choice)}
                    size="sm"
                    variant={presentation.variant}
                  >
                    <Icon /> {presentation.label}
                  </Button>
                );
              })}
            </div>
            {responseMutation.error ? (
              <p className="mt-2 text-xs text-destructive">
                {responseMutation.error.message}
              </p>
            ) : null}
          </>
        )}
      </div>
    </section>
  );
}

function DecisionRecordedState({
  response,
}: {
  response: NonNullable<ReturnType<typeof parseDecisionResponse>>;
}) {
  const presentation = choicePresentation[response.decision];
  const Icon = presentation.icon;
  return (
    <div
      className="flex items-start gap-3 rounded-xl border border-emerald-500/30 bg-emerald-500/8 p-3"
      data-testid="decision-receipt-card"
    >
      <span className="rounded-full bg-emerald-500/15 p-2 text-emerald-700 dark:text-emerald-300">
        <Icon className="size-4" />
      </span>
      <div>
        <p className="text-sm font-semibold">{presentation.outcome}</p>
        <p className="text-xs text-muted-foreground">
          Durable Buzz receipt · SHADOW / NOT DELIVERED
        </p>
        {response.note ? <p className="mt-1 text-sm">{response.note}</p> : null}
      </div>
    </div>
  );
}

export function DecisionReceiptCard({ message }: { message: TimelineMessage }) {
  const response = parseDecisionResponse(message.tags ?? []);
  if (!response) {
    return (
      <p className="text-sm text-destructive">Invalid decision receipt.</p>
    );
  }
  return <DecisionRecordedState response={response} />;
}
