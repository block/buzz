import "./conversation.css";
import { IconAlertCircle, IconMessageCircle } from "@tabler/icons-react";
import { Button } from "@/shared/ui/Button";
import { ShimmerText } from "@/shared/ui/ShimmerText";

/** Empty, loading, and failed reads are distinct; actions belong to the caller. */
export function ConversationState({
  state,
  onRetry,
}: {
  state: "loading" | "empty" | "failed" | "read-only";
  onRetry?: () => void;
}) {
  const copy = {
    loading: ["Loading conversation", "Your messages will appear here."],
    empty: [
      "A place to work things through",
      "Send a message and bring an agent into the conversation.",
    ],
    failed: [
      "Couldn’t load this conversation",
      "Try again to load the messages.",
    ],
    "read-only": [
      "You can read this conversation",
      "You don’t have permission to send messages here.",
    ],
  }[state];
  return (
    <div
      className="conversation-state"
      role={state === "failed" ? "alert" : "status"}
    >
      {state === "failed" ? (
        <IconAlertCircle size={20} aria-hidden="true" />
      ) : (
        <IconMessageCircle size={20} aria-hidden="true" />
      )}
      <p className="text-body text-primary">
        {state === "loading" ? <ShimmerText>{copy[0]}</ShimmerText> : copy[0]}
      </p>
      <p className="text-body-sm text-secondary">{copy[1]}</p>
      {state === "failed" && onRetry ? (
        <Button size="compact" onClick={onRetry}>
          Retry loading
        </Button>
      ) : null}
    </div>
  );
}
