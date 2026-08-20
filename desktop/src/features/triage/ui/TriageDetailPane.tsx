import { ArrowUpCircle, Check, ExternalLink, Sparkles, X } from "lucide-react";

import type { TriageSuggestion } from "@/features/triage/api";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";

const timestampFormatter = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

type TriageDetailPaneProps = {
  isAdopting: boolean;
  onAdopt: (suggestion: TriageSuggestion) => void;
  onDismiss: (suggestion: TriageSuggestion) => void;
  onOpenThread: (suggestion: TriageSuggestion) => void;
  onPromote: (suggestion: TriageSuggestion) => void;
  suggestion: TriageSuggestion | null;
};

export function TriageDetailPane({
  isAdopting,
  onAdopt,
  onDismiss,
  onOpenThread,
  onPromote,
  suggestion,
}: TriageDetailPaneProps) {
  if (!suggestion) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <p className="text-sm text-muted-foreground">
          Select an item to see why it was triaged this way.
        </p>
      </div>
    );
  }

  const isNoise = suggestion.verdict === "noise";

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-5">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <span className="text-base font-semibold text-foreground">
          {suggestion.authorLabel ?? "Unknown sender"}
        </span>
        {suggestion.authorPubkey ? (
          <span className="text-2xs text-muted-foreground">
            {truncatePubkey(suggestion.authorPubkey)}
          </span>
        ) : null}
        {suggestion.channelName ? (
          <Badge variant="outline">#{suggestion.channelName}</Badge>
        ) : null}
        {suggestion.isDm ? <Badge variant="info">DM</Badge> : null}
        {suggestion.isMention ? <Badge variant="info">Mention</Badge> : null}
        {suggestion.createdAt ? (
          <span className="text-2xs text-muted-foreground">
            {timestampFormatter.format(new Date(suggestion.createdAt * 1_000))}
          </span>
        ) : null}
      </div>

      <div className="mt-4 rounded-lg border border-border/60 bg-muted/30 p-3">
        <div className="flex items-center gap-1.5">
          {suggestion.learned ? (
            <Sparkles className="h-3.5 w-3.5 text-primary" />
          ) : null}
          <span className="text-2xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">
            {suggestion.learned ? "Learned from you" : "Why this was triaged"}
          </span>
        </div>
        <p className="mt-1.5 text-sm text-foreground">{suggestion.reason}</p>
        <p className="mt-1 text-2xs text-muted-foreground">
          Confidence {Math.round(suggestion.confidence * 100)}%
        </p>
      </div>

      {suggestion.content.trim() ? (
        <Markdown
          className="mt-4 text-base leading-6 text-foreground"
          content={suggestion.content}
          interactive={false}
        />
      ) : null}

      <div className="mt-5 flex flex-wrap gap-2">
        <Button
          onClick={() => onOpenThread(suggestion)}
          size="sm"
          type="button"
          variant="outline"
        >
          <ExternalLink className="h-4 w-4" />
          Open thread
        </Button>

        {isNoise ? (
          <Button
            data-testid="triage-promote"
            onClick={() => onPromote(suggestion)}
            size="sm"
            type="button"
            variant="outline"
          >
            <ArrowUpCircle className="h-4 w-4" />
            This is important
          </Button>
        ) : (
          <>
            <Button
              data-testid="triage-adopt"
              disabled={isAdopting}
              onClick={() => onAdopt(suggestion)}
              size="sm"
              type="button"
            >
              <Check className="h-4 w-4" />
              Add to todos
            </Button>
            <Button
              data-testid="triage-dismiss"
              onClick={() => onDismiss(suggestion)}
              size="sm"
              type="button"
              variant="ghost"
            >
              <X className="h-4 w-4" />
              Not important
            </Button>
          </>
        )}
      </div>
    </div>
  );
}
