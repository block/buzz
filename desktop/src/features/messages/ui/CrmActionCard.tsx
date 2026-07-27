import * as React from "react";
import { Check, Copy, X } from "lucide-react";

import type { TimelineReaction } from "@/features/messages/types";
import {
  extractCrmRedditDraft,
  type CrmActionCard as CrmAction,
} from "@/features/messages/ui/crmActionCardParser";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { Button } from "@/shared/ui/button";

const APPROVE_EMOJI = "✅";
const CANCEL_EMOJI = "❌";
const LEAD_CATEGORY_CHOICES = [
  { label: "Interested", reaction: "👍" },
  { label: "Meeting Request", reaction: "📅" },
  { label: "Information Request", reaction: "ℹ️" },
  { label: "Not Interested", reaction: "👎" },
  { label: "Out Of Office", reaction: "🕒" },
  { label: "Do Not Contact", reaction: "⛔" },
  { label: "Wrong Person", reaction: "🔀" },
] as const;

export function CrmActionCard({
  action,
  canToggle,
  pending,
  reactions,
  onSelect,
}: {
  action: CrmAction;
  canToggle: boolean;
  pending: boolean;
  reactions: TimelineReaction[];
  onSelect: (emoji: string) => Promise<void>;
}) {
  const [selectedReaction, setSelectedReaction] = React.useState<string | null>(null);
  const expiresAt = Date.parse(action.expiresAt);
  const expired = !Number.isFinite(expiresAt) || Date.now() >= expiresAt;
  const decided = reactions.some(
    (reaction) =>
      reaction.reactedByCurrentUser &&
      (reaction.emoji === APPROVE_EMOJI ||
        reaction.emoji === CANCEL_EMOJI ||
        LEAD_CATEGORY_CHOICES.some((choice) => choice.reaction === reaction.emoji)),
  );
  const disabled = !canToggle || pending || expired || decided;
  const redditDraft = action.actionType === "reddit_mark_posted"
    ? extractCrmRedditDraft(action.content)
    : null;

  if (action.actionType === "lead_categorize") {
    return (
      <div className="my-2 max-w-md rounded-lg border border-input/50 bg-muted/20 p-3">
        <p className="text-sm font-medium">Categorize lead</p>
        <div className="mt-3 flex flex-wrap gap-2">
          {LEAD_CATEGORY_CHOICES.map((choice) => (
            <Button
              disabled={disabled}
              key={choice.reaction}
              onClick={() => setSelectedReaction(choice.reaction)}
              size="sm"
              type="button"
              variant={selectedReaction === choice.reaction ? "default" : "outline"}
            >
              {choice.label}
            </Button>
          ))}
        </div>
        <div className="mt-3 flex gap-2">
          <Button
            disabled={disabled || !selectedReaction}
            onClick={() => selectedReaction && void onSelect(selectedReaction)}
            size="sm"
            type="button"
          >
            <Check aria-hidden="true" />
            Record category
          </Button>
          <Button
            disabled={disabled}
            onClick={() => void onSelect(CANCEL_EMOJI)}
            size="sm"
            type="button"
            variant="outline"
          >
            <X aria-hidden="true" />
            Cancel
          </Button>
        </div>
      </div>
    );
  }

  if (action.actionType === "outreach_approve") {
    return (
      <div className="my-2 max-w-md rounded-lg border border-input/50 bg-muted/20 p-3">
        <p className="text-sm font-medium">Review outreach draft</p>
        <div className="mt-3 flex gap-2">
          <Button disabled={disabled} onClick={() => void onSelect(APPROVE_EMOJI)} size="sm" type="button">
            <Check aria-hidden="true" />
            Approve and send
          </Button>
          <Button disabled={disabled} onClick={() => void onSelect(CANCEL_EMOJI)} size="sm" type="button" variant="outline">
            <X aria-hidden="true" />
            Reject draft
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="my-2 max-w-md rounded-lg border border-input/50 bg-muted/20 p-3">
      <p className="text-sm font-medium">
        Mark Reddit draft as posted
      </p>
      <div className="mt-3 flex gap-2">
        {redditDraft ? (
          <Button
            disabled={pending}
            onClick={() => copyTextToClipboard(redditDraft, "Draft copied")}
            size="sm"
            type="button"
            variant="outline"
          >
            <Copy aria-hidden="true" />
            Copy draft
          </Button>
        ) : null}
        <Button
          disabled={disabled}
          onClick={() => void onSelect(APPROVE_EMOJI)}
          size="sm"
          type="button"
        >
          <Check aria-hidden="true" />
          Approve
        </Button>
        <Button
          disabled={disabled}
          onClick={() => void onSelect(CANCEL_EMOJI)}
          size="sm"
          type="button"
          variant="outline"
        >
          <X aria-hidden="true" />
          Cancel
        </Button>
      </div>
    </div>
  );
}
