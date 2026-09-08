import { IconAlertCircle, IconHash, IconRefresh } from "@tabler/icons-react";
import { useCallback, useMemo } from "react";
import { useConversation } from "@/features/conversation/useConversation";
import { ConversationComposerDock } from "@/features/conversation/ui/ConversationComposerDock";
import { Button } from "@/shared/ui/Button";
import { ConversationHeader } from "@/shared/ui/ConversationHeader";
import type { AgentTurn } from "@/features/agent-activity/types";
import type { Channel, Identity, Message, Participant } from "../types";
import { AgentActivity } from "./AgentActivity";
import { ParticipantDialog } from "./ParticipantDialog";

function displayName(
  pubkey: string,
  participants: Participant[],
  identity: Identity,
) {
  if (pubkey === identity.pubkey) return identity.displayName;
  return (
    participants.find((participant) => participant.pubkey === pubkey)
      ?.displayName ?? "Unknown"
  );
}

function MessageRow({
  message,
  participants,
  identity,
  onRetry,
}: {
  message: Message;
  participants: Participant[];
  identity: Identity;
  onRetry?: () => void;
}) {
  const name = displayName(message.pubkey, participants, identity);
  const isSelf = message.pubkey === identity.pubkey;
  const isAgent = participants.some(
    (participant) =>
      participant.pubkey === message.pubkey && participant.isAgent,
  );
  return (
    <article className="message-row" data-self={isSelf || undefined}>
      <div
        className="message-avatar"
        data-agent={isAgent || undefined}
        aria-hidden="true"
      >
        {name.slice(0, 1)}
      </div>
      <div className="min-w-0 flex-1">
        <header className="message-meta">
          <span className="text-body text-primary">{name}</span>
          {isAgent ? <span className="agent-label">Agent</span> : null}
          <time className="text-body-sm text-tertiary">
            {new Date(message.createdAt * 1000).toLocaleTimeString([], {
              hour: "numeric",
              minute: "2-digit",
            })}
          </time>
        </header>
        <p className="message-content text-body text-primary">
          {message.content}
        </p>
        {message.pending ? (
          <div className="mt-1 text-body-sm text-tertiary" role="status">
            <span>
              {message.pending === "creating"
                ? "Creating Session"
                : message.pending === "waiting"
                  ? "Waiting for connection"
                  : message.pending === "failed"
                    ? (message.error ?? "Not sent")
                    : "Sending"}
            </span>
            {message.pending === "failed" && onRetry ? (
              <button type="button" className="quiet-button" onClick={onRetry}>
                Retry send
              </button>
            ) : null}
          </div>
        ) : null}
      </div>
    </article>
  );
}

export function SessionView({
  channel,
  identity,
  turns,
  mode,
  onStartSession,
  draft,
  onDraftChange,
}: {
  channel: Channel;
  identity: Identity;
  turns: AgentTurn[];
  mode: "channel" | "session";
  onStartSession?: () => void;
  draft: string;
  onDraftChange: (value: string) => void;
}) {
  const { messages, participants, loading, error, refresh, retrySend, send } =
    useConversation(channel, identity);

  const refreshParticipants = useCallback(async () => {
    await refresh();
  }, [refresh]);

  const timeline = useMemo(() => {
    const channelTurns = turns.filter((turn) =>
      turn.key.startsWith(`${channel.id}:`),
    );
    const items: (
      | { key: string; timestamp: number; type: "message"; message: Message }
      | { key: string; timestamp: number; type: "turn"; turn: AgentTurn }
    )[] = [
      ...messages.map((message) => ({
        key: `message:${message.id}`,
        timestamp: message.createdAt * 1000,
        type: "message" as const,
        message,
      })),
      ...channelTurns.map((turn) => ({
        key: `turn:${turn.key}`,
        timestamp: Date.parse(turn.items[0]?.timestamp ?? "") || Date.now(),
        type: "turn" as const,
        turn,
      })),
    ];
    return items.sort((left, right) => left.timestamp - right.timestamp);
  }, [channel.id, messages, turns]);

  return (
    <main className="session-view">
      <ConversationHeader
        icon={<IconHash size={16} stroke={1.7} />}
        title={channel.name}
        metadata={
          mode === "session" ? (
            <span className="device-label">On this device</span>
          ) : undefined
        }
        actions={
          <>
            <ParticipantDialog
              channelId={channel.id}
              participants={participants}
              onChanged={refreshParticipants}
            />
            {mode === "channel" && onStartSession ? (
              <Button variant="quiet" size="compact" onClick={onStartSession}>
                Start Session
              </Button>
            ) : null}
          </>
        }
      />
      <div className="session-scroll">
        <div className="conversation-column">
          {loading && messages.length === 0 ? (
            <p className="state-note text-body text-secondary">
              Loading conversation…
            </p>
          ) : null}
          {error ? (
            <div className="error-state" role="alert">
              <IconAlertCircle size={18} stroke={1.6} aria-hidden="true" />
              <span>{error}</span>
              <button type="button" onClick={() => void refresh()}>
                <IconRefresh size={15} stroke={1.6} aria-hidden="true" /> Retry
              </button>
            </div>
          ) : null}
          {timeline.map((item) =>
            item.type === "message" ? (
              <MessageRow
                key={item.key}
                message={item.message}
                participants={participants}
                identity={identity}
                onRetry={
                  item.message.pending === "failed"
                    ? () => void retrySend(item.message.id)
                    : undefined
                }
              />
            ) : (
              <AgentActivity key={item.key} turn={item.turn} />
            ),
          )}
        </div>
      </div>
      <ConversationComposerDock
        draft={draft}
        onDraftChange={onDraftChange}
        onSend={send}
        placeholder="Reply in this session"
        turns={turns}
      />
    </main>
  );
}
