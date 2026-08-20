import { Send } from "lucide-react";
import * as React from "react";

import {
  DECISION_CARD_CHOICES,
  publishDecisionCard,
} from "@/features/decision-cards/lib/decisionCards";
import { Button } from "@/shared/ui/button";

const shadowCardRecordUrl =
  "https://github.com/Go2Stone/freightman/issues/1258";

export function DecisionCardDevAction({
  channelId,
  recipientPubkeys = [],
}: {
  channelId: string;
  recipientPubkeys?: string[];
}) {
  const [status, setStatus] = React.useState<
    "idle" | "sending" | "sent" | "error"
  >("idle");
  const [errorMessage, setErrorMessage] = React.useState("");

  const publish = React.useCallback(async () => {
    setStatus("sending");
    setErrorMessage("");
    try {
      await publishDecisionCard({
        channelId,
        recipientPubkeys,
        payload: {
          schema_version: 1,
          card_id: crypto.randomUUID(),
          title: "Case #625 — approve the corrected redraft",
          situation:
            "A corrected wording draft is ready for the shadow approval proof.",
          recommendation:
            "Approve the corrected redraft for the shadow replay.",
          proposed_action:
            "Record one signed Buzz decision and pass no external send to Stomaton.",
          risk: "This is a test only: no email, WhatsApp, customer action, money action, or production write.",
          record_url: shadowCardRecordUrl,
          choices: [...DECISION_CARD_CHOICES],
          expires_at: Math.floor(Date.now() / 1_000) + 86_400,
          shadow: true,
        },
      });
      setStatus("sent");
    } catch (error) {
      setStatus("error");
      setErrorMessage(error instanceof Error ? error.message : String(error));
    }
  }, [channelId, recipientPubkeys]);

  return (
    <div className="flex items-center gap-1">
      <Button
        aria-label="Send shadow decision card"
        data-testid="decision-card-dev-send"
        disabled={status === "sending" || status === "sent"}
        onClick={() => void publish()}
        size="sm"
        type="button"
        variant="outline"
      >
        <Send /> {status === "sent" ? "Card sent" : "Send shadow card"}
      </Button>
      {status === "error" ? (
        <span
          aria-live="polite"
          className="max-w-64 truncate text-2xs text-destructive"
          data-testid="decision-card-dev-error"
          title={errorMessage}
        >
          Failed: {errorMessage}
        </span>
      ) : null}
    </div>
  );
}
