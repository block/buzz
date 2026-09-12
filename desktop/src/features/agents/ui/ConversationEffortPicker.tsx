import * as React from "react";
import { switchManagedAgentEffort } from "@/shared/api/agentControl";
import {
  getAgentSessionConfigs,
  subscribeControlResults,
} from "../observerRelayStore";
import {
  conversationEfforts,
  matchingEffortStatus,
  type EffortRequest,
} from "../lib/conversationEffort";
import type { ObserverEvent } from "./agentSessionTypes";
import { PersonaDropdownField } from "./PersonaDropdownField";

const RESULT_TEXT: Record<string, string> = {
  applied: "Thinking level applied to this conversation.",
  queued: "Queued — applies after the current response, before the next one.",
  rejected: "The adapter rejected this level. The previous level is unchanged.",
  unsupported: "This model does not support that level. Choose another level.",
  stale_session:
    "This conversation’s session has ended. Open its latest activity and try again.",
  unavailable: "Live thinking levels are unavailable for this conversation.",
  busy: "Another change is pending. Wait for its result before trying again.",
  expired: "The change expired before it could apply. Choose the level again.",
  unconfirmed:
    "The agent did not confirm this change. Check its reported level before continuing.",
  invalid_request: "The agent could not accept this request.",
};

export function ConversationEffortPicker({
  pubkey,
  channelId,
  events,
}: {
  pubkey: string;
  channelId: string | null;
  events: readonly ObserverEvent[];
}) {
  const retained = getAgentSessionConfigs(pubkey);
  const choices = React.useMemo(
    () => conversationEfforts([...retained, ...events], channelId),
    [retained, events, channelId],
  );
  const [selectedSession, setSelectedSession] = React.useState("");
  const [pending, setPending] = React.useState<EffortRequest | null>(null);
  const [message, setMessage] = React.useState<string | null>(null);
  const selection =
    choices.find(
      (choice) => `${choice.channelId}:${choice.sessionId}` === selectedSession,
    ) ?? (choices.length === 1 ? choices[0] : undefined);
  const subscription = React.useRef<() => void>(() => {});
  const id = React.useId();
  React.useEffect(() => () => subscription.current(), []);
  React.useEffect(() => {
    if (
      pending &&
      !choices.some(
        (choice) =>
          choice.channelId === pending.channelId &&
          choice.sessionId === pending.sessionId &&
          choice.sessionToken === pending.sessionToken,
      )
    ) {
      subscription.current();
      setPending(null);
      setMessage(RESULT_TEXT.stale_session);
    }
  }, [choices, pending]);

  async function change(effort: string) {
    if (!selection || pending) return;
    const request = {
      requestId: crypto.randomUUID(),
      channelId: selection.channelId,
      sessionId: selection.sessionId,
      sessionToken: selection.sessionToken,
      effort,
    };
    setPending(request);
    setMessage("Requesting thinking level change…");
    let timeout: ReturnType<typeof setTimeout>;
    let disposed = false;
    const deliveryTimeout = setTimeout(() => {
      setMessage(
        "Waiting for the agent to acknowledge this change. The applied level is still unconfirmed.",
      );
    }, 8_000);
    const unsubscribe = subscribeControlResults(pubkey, (frame) => {
      const status = matchingEffortStatus(frame, request);
      if (!status) return;
      clearTimeout(deliveryTimeout);
      setMessage(RESULT_TEXT[status] ?? RESULT_TEXT.unconfirmed);
      if (status !== "queued") {
        setPending(null);
        subscription.current();
      }
    });
    subscription.current = () => {
      disposed = true;
      unsubscribe();
      clearTimeout(timeout);
      clearTimeout(deliveryTimeout);
    };
    timeout = setTimeout(() => {
      setMessage(RESULT_TEXT.unconfirmed);
      setPending(null);
      subscription.current();
    }, 310_000);
    try {
      await switchManagedAgentEffort(pubkey, request);
    } catch {
      if (disposed) return;
      subscription.current();
      setPending(null);
      setMessage(
        "Could not deliver the change. The applied level is unconfirmed.",
      );
    }
  }

  if (choices.length === 0) return null;
  return (
    <div
      className="space-y-2 border-b border-border/55 py-3"
      data-testid="conversation-effort-picker"
    >
      {choices.length > 1 ? (
        <>
          <label className="text-xs font-medium" htmlFor={`${id}-conversation`}>
            Conversation
          </label>
          <PersonaDropdownField
            id={`${id}-conversation`}
            disabled={pending !== null}
            value={
              selection ? `${selection.channelId}:${selection.sessionId}` : ""
            }
            onValueChange={setSelectedSession}
            placeholder="Choose a conversation"
            options={choices.map((choice, index) => ({
              value: `${choice.channelId}:${choice.sessionId}`,
              label: `${choice.label ?? `Conversation ${index + 1}`} · ${new Date(choice.timestamp).toLocaleString()}`,
            }))}
          />
        </>
      ) : null}
      <label className="block text-sm font-medium" htmlFor={`${id}-effort`}>
        Thinking level
      </label>
      <PersonaDropdownField
        id={`${id}-effort`}
        disabled={!selection || pending !== null}
        value={selection?.value ?? ""}
        onValueChange={(value) => {
          void change(value);
        }}
        options={selection?.options ?? []}
        placeholder="Choose a conversation first"
      />
      <p className="text-xs text-muted-foreground" role="status">
        {message ??
          "For this conversation. A change applies after the current response; saved defaults stay unchanged."}
      </p>
    </div>
  );
}
