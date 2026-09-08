import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  Channel,
  Identity,
  Message,
  Participant,
} from "@/features/sessions/types";
import { runtime } from "@/shared/runtime/client";
import { mergeMessages, projectChannelWindow } from "./channelWindow";

type SendOperation = {
  localId: string;
  content: string;
  message: Message;
};

function messageError(caught: unknown) {
  return caught instanceof Error ? caught.message : String(caught);
}

/**
 * The behavioral owner of one shared conversation.
 *
 * History is fetched through the relay's window contract. Live delivery is a
 * separate overlay, reconciled with a fresh head read after subscription setup
 * settles so the interval between the two cannot lose a message.
 */
export function useConversation(channel: Channel, identity: Identity) {
  const generation = useRef(0);
  const [history, setHistory] = useState<Message[]>([]);
  const [live, setLive] = useState<Message[]>([]);
  const [participants, setParticipants] = useState<Participant[]>([]);
  const [operations, setOperations] = useState<SendOperation[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const loadHead = useCallback(
    async (expectedGeneration: number) => {
      const [window, nextParticipants] = await Promise.all([
        runtime
          .channelWindow({ channelId: channel.id })
          .then((events) =>
            projectChannelWindow(events, { channelId: channel.id }),
          ),
        runtime.participants(channel.id),
      ]);
      if (generation.current !== expectedGeneration) return;
      setHistory(window.messages);
      setParticipants(nextParticipants);
      setError(null);
    },
    [channel.id],
  );

  const refresh = useCallback(async () => {
    const expectedGeneration = generation.current;
    setLoading(true);
    try {
      await loadHead(expectedGeneration);
    } catch (caught) {
      if (generation.current === expectedGeneration)
        setError(messageError(caught));
    } finally {
      if (generation.current === expectedGeneration) setLoading(false);
    }
  }, [loadHead]);

  useEffect(() => {
    const expectedGeneration = generation.current + 1;
    generation.current = expectedGeneration;
    let disposed = false;
    let stop: (() => void) | undefined;

    setHistory([]);
    setLive([]);
    setParticipants([]);
    setOperations([]);
    setError(null);
    setLoading(true);

    const settleInitialLoad = async () => {
      try {
        await loadHead(expectedGeneration);
      } catch (caught) {
        if (!disposed && generation.current === expectedGeneration) {
          setError(messageError(caught));
        }
      } finally {
        if (!disposed && generation.current === expectedGeneration) {
          setLoading(false);
        }
      }
    };

    void settleInitialLoad();
    void runtime
      .subscribeMessages(channel.id, (incoming) => {
        if (disposed || generation.current !== expectedGeneration) return;
        setLive((current) => mergeMessages(current, [incoming]));
      })
      .then(
        (cleanup) => {
          if (disposed) {
            cleanup();
            return;
          }
          stop = cleanup;
          // The live subscription begins at now. Re-read the head after it is
          // armed to close the history/subscription establishment interval.
          return loadHead(expectedGeneration).catch((caught) => {
            if (!disposed && generation.current === expectedGeneration) {
              setError(`Live updates unavailable: ${messageError(caught)}`);
            }
          });
        },
        (caught) => {
          if (!disposed && generation.current === expectedGeneration) {
            setError(`Live updates unavailable: ${messageError(caught)}`);
          }
          // The head is still authoritative when the live socket is not.
          return loadHead(expectedGeneration).catch(() => undefined);
        },
      );

    return () => {
      disposed = true;
      generation.current += 1;
      stop?.();
    };
  }, [channel.id, loadHead]);

  const messages = useMemo(() => {
    const accepted = mergeMessages(history, live);
    const pending = operations.map(({ message }) => message);
    return [...accepted, ...pending].sort(
      (left, right) =>
        left.createdAt - right.createdAt || left.id.localeCompare(right.id),
    );
  }, [history, live, operations]);

  const send = useCallback(
    async (content: string, retryLocalId?: string) => {
      const expectedGeneration = generation.current;
      const existing = retryLocalId
        ? operations.find((operation) => operation.localId === retryLocalId)
        : undefined;
      const localId = existing?.localId ?? `pending-${crypto.randomUUID()}`;
      const payload = existing?.content ?? content;
      const pending: Message = {
        id: localId,
        pubkey: identity.pubkey,
        content: payload,
        createdAt: Math.floor(Date.now() / 1000),
        kind: 9,
        tags: [["h", channel.id]],
        pending: navigator.onLine ? "sending" : "waiting",
      };
      setOperations((current) => [
        ...current.filter((operation) => operation.localId !== localId),
        { localId, content: payload, message: pending },
      ]);

      try {
        const accepted = await runtime.sendMessage(channel.id, payload);
        if (generation.current !== expectedGeneration) return;
        setOperations((current) =>
          current.filter((operation) => operation.localId !== localId),
        );
        setLive((current) => mergeMessages(current, [accepted]));
      } catch (caught) {
        if (generation.current !== expectedGeneration) return;
        setOperations((current) =>
          current.map((operation) =>
            operation.localId === localId
              ? {
                  ...operation,
                  message: {
                    ...operation.message,
                    pending: "failed",
                    error: messageError(caught),
                  },
                }
              : operation,
          ),
        );
        throw caught;
      }
    },
    [channel.id, identity.pubkey, operations],
  );

  const retrySend = useCallback(
    async (localId: string) => {
      const operation = operations.find(
        (current) => current.localId === localId,
      );
      if (operation) await send(operation.content, localId);
    },
    [operations, send],
  );

  return { messages, participants, loading, error, refresh, send, retrySend };
}
