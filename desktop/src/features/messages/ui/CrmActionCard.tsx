import * as React from "react";
import {
  Building2,
  Check,
  Copy,
  Pencil,
  ShieldBan,
  Trash2,
  X,
} from "lucide-react";

import type { TimelineReaction } from "@/features/messages/types";
import {
  extractCrmRedditDraft,
  type CrmActionCard as CrmAction,
} from "@/features/messages/ui/crmActionCardParser";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { Button } from "@/shared/ui/button";

const APPROVE_EMOJI = "✅";
const CANCEL_EMOJI = "❌";
const EDIT_EMOJI = "✏️";
const LEAD_CATEGORY_CHOICES = [
  { label: "Interested", reaction: "👍" },
  { label: "Meeting Request", reaction: "📅" },
  { label: "Information Request", reaction: "ℹ️" },
  { label: "Not Interested", reaction: "👎" },
  { label: "Out Of Office", reaction: "🕒" },
  { label: "Do Not Contact", reaction: "⛔" },
  { label: "Wrong Person", reaction: "🔀" },
] as const;
const LEAD_CONTROL_CHOICES = [
  { icon: ShieldBan, label: "Block person", reaction: "⛔" },
  { icon: Building2, label: "Block company", reaction: "🏢" },
  { icon: Trash2, label: "Remove from campaign", reaction: "🗑️" },
] as const;
const CALENDAR_SLOT_REACTIONS = new Set(["1️⃣", "2️⃣", "3️⃣"]);

function isFinalDecisionReaction(
  actionType: CrmAction["actionType"],
  emoji: string,
): boolean {
  switch (actionType) {
    case "reddit_mark_posted":
      return emoji === APPROVE_EMOJI || emoji === CANCEL_EMOJI;
    case "lead_categorize":
      return (
        emoji === CANCEL_EMOJI ||
        LEAD_CATEGORY_CHOICES.some((choice) => choice.reaction === emoji)
      );
    case "outreach_approve":
      return emoji === APPROVE_EMOJI || emoji === CANCEL_EMOJI;
    case "calendar_book":
      return emoji === CANCEL_EMOJI || CALENDAR_SLOT_REACTIONS.has(emoji);
    case "lead_control":
      return emoji === APPROVE_EMOJI;
  }
}

export function CrmActionCard({
  action,
  canToggle,
  pending,
  reactions,
  onSelect,
  onChooseLeadControl,
}: {
  action: CrmAction;
  canToggle: boolean;
  pending: boolean;
  reactions: TimelineReaction[];
  onSelect: (emoji: string) => Promise<void>;
  onChooseLeadControl: (
    choices: readonly string[],
    emoji: string,
  ) => Promise<void>;
}) {
  const [selectedReaction, setSelectedReaction] = React.useState<string | null>(
    null,
  );
  const expiresAt = Date.parse(action.expiresAt);
  const expired = !Number.isFinite(expiresAt) || Date.now() >= expiresAt;
  const decided = reactions.some(
    (reaction) =>
      reaction.reactedByCurrentUser &&
      isFinalDecisionReaction(action.actionType, reaction.emoji),
  );
  const disabled = !canToggle || pending || expired || decided;
  const redditDraft =
    action.actionType === "reddit_mark_posted"
      ? extractCrmRedditDraft(action.content)
      : null;
  const calendarSlots =
    action.actionType === "calendar_book" ? (action.calendarSlots ?? []) : [];
  const leadControlReactions =
    action.actionType === "lead_control"
      ? (action.leadControlChoices ??
        LEAD_CONTROL_CHOICES.map((choice) => choice.reaction))
      : [];
  const selectedLeadControl =
    action.actionType === "lead_control"
      ? ([...reactions]
          .reverse()
          .find(
            (reaction) =>
              reaction.reactedByCurrentUser &&
              leadControlReactions.includes(reaction.emoji),
          )?.emoji ?? null)
      : null;

  if (action.actionType === "lead_control") {
    const availableLeadControlChoices = LEAD_CONTROL_CHOICES.filter((choice) =>
      leadControlReactions.includes(choice.reaction),
    );
    return (
      <div className="my-2 max-w-md rounded-lg border border-input/50 bg-muted/20 p-3">
        <p className="text-sm font-medium">Lead safeguards</p>
        <div className="mt-3 flex flex-wrap gap-2">
          {availableLeadControlChoices.map((choice) => {
            const Icon = choice.icon;
            const selected = selectedLeadControl === choice.reaction;

            return (
              <Button
                aria-pressed={selected}
                disabled={disabled}
                key={choice.reaction}
                onClick={() => {
                  if (!selected) {
                    void onChooseLeadControl(
                      availableLeadControlChoices.map((item) => item.reaction),
                      choice.reaction,
                    );
                  }
                }}
                size="sm"
                type="button"
                variant={selected ? "default" : "outline"}
              >
                <Icon aria-hidden="true" />
                {choice.label}
              </Button>
            );
          })}
        </div>
        {selectedLeadControl ? (
          <div className="mt-3 flex flex-wrap gap-2">
            <Button
              disabled={disabled}
              onClick={() => void onSelect(APPROVE_EMOJI)}
              size="sm"
              type="button"
              variant="destructive"
            >
              <Check aria-hidden="true" />
              Apply change
            </Button>
          </div>
        ) : null}
      </div>
    );
  }

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
              variant={
                selectedReaction === choice.reaction ? "default" : "outline"
              }
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

  if (action.actionType === "calendar_book") {
    return (
      <div className="my-2 max-w-md rounded-lg border border-input/50 bg-muted/20 p-3">
        <p className="text-sm font-medium">Book meeting</p>
        <div className="mt-3 flex flex-wrap gap-2">
          {calendarSlots.map((choice) => (
            <Button
              disabled={disabled}
              key={choice.reaction}
              onClick={() => setSelectedReaction(choice.reaction)}
              size="sm"
              type="button"
              variant={
                selectedReaction === choice.reaction ? "default" : "outline"
              }
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
            Confirm booking
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
          <Button
            disabled={disabled}
            onClick={() => void onSelect(APPROVE_EMOJI)}
            size="sm"
            type="button"
          >
            <Check aria-hidden="true" />
            Approve and send
          </Button>
          <Button
            disabled={disabled}
            onClick={() => void onSelect(EDIT_EMOJI)}
            size="sm"
            type="button"
            variant="outline"
          >
            <Pencil aria-hidden="true" />
            Edit draft
          </Button>
          <Button
            disabled={disabled}
            onClick={() => void onSelect(CANCEL_EMOJI)}
            size="sm"
            type="button"
            variant="outline"
          >
            <X aria-hidden="true" />
            Reject draft
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="my-2 max-w-md rounded-lg border border-input/50 bg-muted/20 p-3">
      <p className="text-sm font-medium">Mark Reddit draft as posted</p>
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
