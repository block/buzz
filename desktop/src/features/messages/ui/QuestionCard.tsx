import * as React from "react";
import { HelpCircle } from "lucide-react";
import { toast } from "sonner";

import type { TimelineMessage } from "@/features/messages/types";
import { useIdentityQuery } from "@/shared/api/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_ELICITATION_RESPONSE } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import { Input } from "@/shared/ui/input";

type QuestionCardProps = {
  channelId: string | null;
  className?: string;
  message: TimelineMessage;
};

type ElicitationOption = {
  label: string;
  description?: string;
};

type ElicitationRequest = {
  questionKey?: string;
  header?: string;
  prompt?: string;
  multiSelect: boolean;
  allowCustom: boolean;
  options: ElicitationOption[];
};

function parseElicitationRequest(content: string): ElicitationRequest | null {
  try {
    const parsed = JSON.parse(content) as {
      questionKey?: unknown;
      header?: unknown;
      prompt?: unknown;
      multiSelect?: unknown;
      allowCustom?: unknown;
      options?: unknown;
    };
    const rawOptions = Array.isArray(parsed.options) ? parsed.options : [];
    const options: ElicitationOption[] = [];
    for (const option of rawOptions) {
      if (
        option &&
        typeof option === "object" &&
        typeof (option as { label?: unknown }).label === "string"
      ) {
        const label = (option as { label: string }).label;
        const description = (option as { description?: unknown }).description;
        options.push({
          label,
          description:
            typeof description === "string" ? description : undefined,
        });
      }
    }
    if (options.length === 0) return null;
    return {
      questionKey:
        typeof parsed.questionKey === "string" ? parsed.questionKey : undefined,
      header: typeof parsed.header === "string" ? parsed.header : undefined,
      prompt: typeof parsed.prompt === "string" ? parsed.prompt : undefined,
      multiSelect: parsed.multiSelect === true,
      allowCustom: parsed.allowCustom === true,
      options,
    };
  } catch {
    return null;
  }
}

type AnsweredState = {
  answer: string[];
  custom: string;
};

function parseResponseContent(content: string): AnsweredState | null {
  try {
    const parsed = JSON.parse(content) as {
      answer?: unknown;
      custom?: unknown;
    };
    const answer = Array.isArray(parsed.answer)
      ? parsed.answer.filter(
          (value): value is string => typeof value === "string",
        )
      : typeof parsed.answer === "string" && parsed.answer.length > 0
        ? [parsed.answer]
        : [];
    const custom = typeof parsed.custom === "string" ? parsed.custom : "";
    return { answer, custom };
  } catch {
    return null;
  }
}

function getTag(message: TimelineMessage, name: string): string | undefined {
  return message.tags?.find((tag) => tag[0] === name)?.[1];
}

export function QuestionCard({
  channelId,
  className,
  message,
}: QuestionCardProps) {
  const request = React.useMemo(
    () => parseElicitationRequest(message.body),
    [message.body],
  );
  const ownerPubkey = React.useMemo(() => {
    const tag = getTag(message, "p");
    return tag ? normalizePubkey(tag) : null;
  }, [message]);
  const currentPubkey = useIdentityQuery().data?.pubkey;
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;
  const isOwner = Boolean(
    ownerPubkey &&
      normalizedCurrentPubkey &&
      ownerPubkey === normalizedCurrentPubkey,
  );

  const [selected, setSelected] = React.useState<Set<string>>(() => new Set());
  const [customValue, setCustomValue] = React.useState("");
  const [isSubmitting, setIsSubmitting] = React.useState(false);
  const [answered, setAnswered] = React.useState<AnsweredState | null>(null);

  // Detect an existing owner-authored answer referencing this card. Seeds from
  // the loaded timeline (any 44301 already present) and subscribes for late
  // arrivals, mirroring HuddleAttachment's live-subscription pattern.
  React.useEffect(() => {
    if (!channelId || !ownerPubkey) return;

    let disposed = false;
    let cleanup: (() => void) | null = null;

    function applyResponse(event: RelayEvent) {
      if (disposed) return;
      if (normalizePubkey(event.pubkey ?? "") !== ownerPubkey) return;
      const parsed = parseResponseContent(event.content);
      if (parsed) setAnswered(parsed);
    }

    relayClient
      .subscribeLive(
        {
          kinds: [KIND_ELICITATION_RESPONSE],
          authors: [ownerPubkey],
          "#e": [message.id],
          limit: 1,
        },
        applyResponse,
      )
      .then((dispose) => {
        if (disposed) {
          void dispose();
          return;
        }
        cleanup = () => void dispose();
      })
      .catch((error) => {
        console.error("[QuestionCard] response subscription failed:", error);
      });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, [channelId, message.id, ownerPubkey]);

  if (!request) {
    return (
      <div
        className={cn(
          "mt-2 w-96 max-w-full rounded-2xl border border-border/70 bg-muted/30 px-3 py-2.5",
          className,
        )}
        data-testid="question-card"
        data-state="error"
      >
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <HelpCircle className="h-4 w-4 shrink-0" />
          This question card is missing its details.
        </div>
      </div>
    );
  }

  const toggleSelection = (label: string) => {
    setSelected((current) => {
      const next = new Set(request.multiSelect ? current : []);
      if (next.has(label)) {
        next.delete(label);
      } else {
        next.add(label);
      }
      return next;
    });
  };

  const hasSelection = selected.size > 0;
  const trimmedCustom = customValue.trim();
  const canSubmit =
    isOwner &&
    !answered &&
    !isSubmitting &&
    (hasSelection || trimmedCustom.length > 0);

  async function handleSubmit() {
    if (!channelId || !request) return;
    if (isSubmitting || answered) return;

    const selectedLabels = [...selected];
    const custom = customValue.trim();
    if (selectedLabels.length === 0 && custom.length === 0) return;

    const answer: string | string[] = request.multiSelect
      ? selectedLabels
      : (selectedLabels[0] ?? "");

    setIsSubmitting(true);
    try {
      const event = await signRelayEvent({
        kind: KIND_ELICITATION_RESPONSE,
        content: JSON.stringify({
          action: "accept",
          answer,
          custom,
        }),
        tags: [
          ["h", channelId],
          ["e", message.id, "", "reply"],
        ],
      });
      await relayClient.publishEvent(
        event,
        "Timed out sending your answer.",
        "Failed to send your answer.",
      );
      setAnswered({
        answer: request.multiSelect
          ? selectedLabels
          : selectedLabels.slice(0, 1),
        custom,
      });
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to send your answer.",
      );
    } finally {
      setIsSubmitting(false);
    }
  }

  const isAnswered = answered !== null;
  const interactive = isOwner && !isAnswered;
  const answeredLabels = new Set(answered?.answer ?? []);

  return (
    <div
      className={cn(
        "mt-2 w-96 max-w-full rounded-2xl border border-border/70 bg-muted/30 px-3 py-3",
        className,
      )}
      data-testid="question-card"
      data-state={isAnswered ? "answered" : interactive ? "open" : "readonly"}
    >
      <div className="flex items-start gap-2">
        <HelpCircle className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          {request.header ? (
            <div className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
              {request.header}
            </div>
          ) : null}
          {request.prompt ? (
            <div className="text-sm font-semibold text-foreground">
              {request.prompt}
            </div>
          ) : null}
        </div>
      </div>

      {!isOwner && !isAnswered ? (
        <p className="mt-2 text-2xs text-muted-foreground">
          Question for the owner
        </p>
      ) : null}

      <div className="mt-3 flex flex-col gap-1.5">
        {request.options.map((option) => {
          const isSelected = isAnswered
            ? answeredLabels.has(option.label)
            : selected.has(option.label);
          return (
            <button
              className={cn(
                "flex w-full items-start gap-2 rounded-lg border px-2.5 py-1.5 text-left transition-colors",
                isSelected
                  ? "border-primary/60 bg-primary/10"
                  : "border-border/60 bg-background",
                interactive
                  ? "hover:border-border hover:bg-muted/60"
                  : "cursor-default opacity-70",
              )}
              disabled={!interactive}
              key={option.label}
              onClick={() => toggleSelection(option.label)}
              type="button"
            >
              {request.multiSelect ? (
                <Checkbox
                  checked={isSelected}
                  className="mt-0.5 pointer-events-none"
                  tabIndex={-1}
                />
              ) : null}
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium text-foreground">
                  {option.label}
                </span>
                {option.description ? (
                  <span className="block text-2xs text-muted-foreground">
                    {option.description}
                  </span>
                ) : null}
              </span>
            </button>
          );
        })}
      </div>

      {request.allowCustom ? (
        <div className="mt-2">
          <Input
            aria-label="Other answer"
            disabled={!interactive}
            onChange={(event) => setCustomValue(event.target.value)}
            placeholder="Other…"
            value={isAnswered ? (answered?.custom ?? "") : customValue}
          />
        </div>
      ) : null}

      {interactive ? (
        <div className="mt-3 flex justify-end">
          <Button
            disabled={!canSubmit}
            onClick={() => void handleSubmit()}
            size="sm"
            type="button"
          >
            {isSubmitting ? "Sending…" : "Submit"}
          </Button>
        </div>
      ) : null}

      {isAnswered ? (
        <p className="mt-3 text-2xs text-muted-foreground">Answer submitted</p>
      ) : null}
    </div>
  );
}
