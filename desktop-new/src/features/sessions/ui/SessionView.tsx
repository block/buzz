import { IconAlertCircle, IconHash, IconRefresh } from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { runtime } from "@/shared/runtime/client";
import { reduceActivity } from "../activityProjection";
import type {
  AgentTurn,
  Channel,
  Identity,
  Message,
  ObserverEvent,
  Participant,
} from "../types";
import { AgentActivity } from "./AgentActivity";
import { ParticipantDialog } from "./ParticipantDialog";
import { SessionComposer } from "./SessionComposer";

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
}: {
  message: Message;
  participants: Participant[];
  identity: Identity;
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
          <p className="mt-1 text-body-sm text-tertiary" role="status">
            {message.pending === "creating"
              ? "Creating Session"
              : message.pending === "waiting"
                ? "Waiting for connection"
                : message.pending === "failed"
                  ? (message.error ?? "Not sent")
                  : "Sending"}
          </p>
        ) : null}
      </div>
    </article>
  );
}

export function SessionView({
  channel,
  identity,
  origin,
  turns,
  mode,
  onStartSession,
}: {
  channel: Channel;
  identity: Identity;
  origin?: Channel;
  turns: AgentTurn[];
  mode: "channel" | "session";
  onStartSession?: () => void;
}) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [participants, setParticipants] = useState<Participant[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const refreshParticipants = useCallback(async () => {
    setParticipants(await runtime.participants(channel.id));
  }, [channel.id]);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const [nextMessages] = await Promise.all([
        runtime.messages(channel.id),
        refreshParticipants(),
      ]);
      setMessages((current) => {
        const merged = new Map(current.map((item) => [item.id, item]));
        for (const item of nextMessages) merged.set(item.id, item);
        return [...merged.values()].sort(
          (left, right) => left.createdAt - right.createdAt,
        );
      });
      setError(null);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setLoading(false);
    }
  }, [channel.id, refreshParticipants]);

  useEffect(() => {
    let stop: undefined | (() => void);
    let disposed = false;
    void refresh();
    void runtime
      .subscribeMessages(channel.id, (incoming) => {
        setMessages((current) => {
          if (current.some((item) => item.id === incoming.id)) return current;
          return [...current, incoming].sort(
            (left, right) => left.createdAt - right.createdAt,
          );
        });
      })
      .then((cleanup) => {
        if (disposed) cleanup();
        else stop = cleanup;
      })
      .catch((caught) => {
        if (!disposed) {
          setError(
            caught instanceof Error
              ? `Live updates unavailable: ${caught.message}`
              : "Live updates unavailable.",
          );
        }
      });
    const timer = window.setInterval(() => void refresh(), 30_000);
    return () => {
      disposed = true;
      stop?.();
      window.clearInterval(timer);
    };
  }, [channel.id, refresh]);

  async function send(content: string) {
    const pendingId = `pending-${crypto.randomUUID()}`;
    const pending: Message = {
      id: pendingId,
      pubkey: identity.pubkey,
      content,
      createdAt: Math.floor(Date.now() / 1000),
      kind: 9,
      tags: [["h", channel.id]],
      pending: navigator.onLine ? "sending" : "waiting",
    };
    setMessages((current) => [...current, pending]);
    try {
      const accepted = await runtime.sendMessage(channel.id, content);
      setMessages((current) =>
        current.map((item) => (item.id === pendingId ? accepted : item)),
      );
    } catch (caught) {
      setMessages((current) =>
        current.map((item) =>
          item.id === pendingId
            ? {
                ...item,
                pending: "failed",
                error:
                  caught instanceof Error ? caught.message : String(caught),
              }
            : item,
        ),
      );
      throw caught;
    }
  }

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
      <header className="session-header">
        <div className="session-title-block">
          <div className="flex items-center gap-2">
            <h1 className="text-heading text-primary">
              {mode === "channel" ? `#${channel.name}` : channel.name}
            </h1>
            {mode === "session" ? (
              <span className="device-label">On this device</span>
            ) : null}
          </div>
          {origin ? (
            <span className="origin-label text-body-sm text-secondary">
              <IconHash size={13} stroke={1.6} aria-hidden="true" />
              From {origin.name}
            </span>
          ) : (
            <span className="text-body-sm text-tertiary">
              {mode === "channel"
                ? channel.description || "Channel"
                : "Private Session"}
            </span>
          )}
        </div>
        <div className="session-header-actions">
          <ParticipantDialog
            channelId={channel.id}
            participants={participants}
            onChanged={refreshParticipants}
          />
          {mode === "channel" && onStartSession ? (
            <button
              type="button"
              className="quiet-button"
              onClick={onStartSession}
            >
              Start Session
            </button>
          ) : null}
        </div>
      </header>
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
              />
            ) : (
              <AgentActivity key={item.key} turn={item.turn} />
            ),
          )}
        </div>
      </div>
      <div className="composer-dock">
        <SessionComposer onSend={send} />
      </div>
    </main>
  );
}

export function useAgentTurns() {
  const [projection, setProjection] = useState<Map<string, AgentTurn>>(
    new Map(),
  );
  useEffect(() => {
    let stop: undefined | (() => void);
    void runtime
      .observe((event: ObserverEvent) => {
        setProjection((current) => reduceActivity(current, event));
      })
      .then((cleanup) => {
        stop = cleanup;
      });
    return () => stop?.();
  }, []);
  return [...projection.values()];
}
