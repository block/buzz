import * as React from "react";
import { isTauri } from "@tauri-apps/api/core";

import {
  type TerminalDelivery,
  TerminalConnection,
  type TerminalFrameMessage,
  type TerminalMessage,
} from "./terminalClient";
import {
  TerminalSubstrate,
  type TerminalViewportSize,
} from "./TerminalSubstrate";

type TerminalContext = {
  channelId: string;
  channelName: string;
  threadId: string | null;
  npub: string;
  relayUrl: string;
};

type Session = {
  key: string;
  connection: TerminalConnection | null;
  delivery: TerminalDelivery | null;
  frame: TerminalFrameMessage | undefined;
  title: string;
  closing: boolean;
};

const INITIAL_SIZE: TerminalViewportSize = {
  columns: 80,
  rows: 24,
  pixelWidth: 672,
  pixelHeight: 408,
};

function report(error: unknown) {
  console.error("terminal session failed", error);
}

export function TerminalBootstrap({
  channelId,
  channelName,
  threadId,
  npub,
  relayUrl,
}: {
  channelId: string | null;
  channelName: string | null;
  threadId: string | null;
  npub: string | null;
  relayUrl: string | null;
}) {
  const context =
    channelId && npub && relayUrl
      ? {
          channelId,
          channelName: channelName ?? channelId,
          threadId,
          npub,
          relayUrl,
        }
      : null;
  const contextRef = React.useRef<TerminalContext | null>(context);
  contextRef.current = context;
  const mountedRef = React.useRef(true);
  const sizeRef = React.useRef(INITIAL_SIZE);
  const resizeChainRef = React.useRef(Promise.resolve());
  const [sessions, setSessions] = React.useState<Session[]>([]);
  const [activeKey, setActiveKey] = React.useState<string | null>(null);
  const [available, setAvailable] = React.useState(() => isTauri());
  const acknowledgedSequenceRef = React.useRef(new Map<string, number>());
  const sessionsRef = React.useRef(sessions);
  sessionsRef.current = sessions;

  const fail = React.useCallback((error: unknown) => {
    report(error);
    setAvailable(false);
  }, []);

  const removeSession = React.useCallback((key: string) => {
    setSessions((current) => current.filter((session) => session.key !== key));
    setActiveKey((current) => {
      if (current !== key) return current;
      const remaining = sessionsRef.current.filter(
        (session) => session.key !== key,
      );
      return remaining.at(-1)?.key ?? null;
    });
  }, []);

  const createSession = React.useCallback(() => {
    const spawnContext = contextRef.current;
    if (!available || !spawnContext) return;
    const key = crypto.randomUUID();
    const initial: Session = {
      key,
      connection: null,
      delivery: null,
      frame: undefined,
      title: "SHELL",
      closing: false,
    };
    setSessions((current) => [...current, initial]);
    setActiveKey(key);

    const update = (apply: (session: Session) => Session) => {
      if (!mountedRef.current) return;
      setSessions((current) =>
        current.map((session) =>
          session.key === key ? apply(session) : session,
        ),
      );
    };
    const onMessage = (
      message: Exclude<TerminalMessage, { type: "frame" }>,
    ) => {
      if (message.type === "exit") {
        removeSession(key);
      } else if (message.type === "title") {
        update((session) => ({
          ...session,
          title: message.payload || "SHELL",
        }));
      } else if (message.type === "resetTitle") {
        update((session) => ({ ...session, title: "SHELL" }));
      }
    };
    const onFrame = (delivery: TerminalDelivery) => {
      update((session) => ({ ...session, delivery, frame: delivery.frame }));
    };

    const size = sizeRef.current;
    void TerminalConnection.attach(
      {
        ...spawnContext,
        threadId: spawnContext.threadId ?? undefined,
        ...size,
      },
      onMessage,
      onFrame,
    )
      .then((connection) => {
        if (!mountedRef.current) return connection.detach();
        update((session) => ({ ...session, connection }));
        if (sizeRef.current !== size) {
          const currentSize = sizeRef.current;
          resizeChainRef.current = resizeChainRef.current
            .then(async () => {
              const viewport = await connection.resize(
                currentSize.columns,
                currentSize.rows,
                currentSize.pixelWidth,
                currentSize.pixelHeight,
              );
              await connection.viewportReady(viewport);
            })
            .catch(fail);
        }
      })
      .catch((error) => {
        removeSession(key);
        fail(error);
      });
  }, [available, fail, removeSession]);

  React.useEffect(() => {
    if (available && context && sessions.length === 0) createSession();
  }, [available, context, createSession, sessions.length]);

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      for (const session of sessionsRef.current) {
        void session.connection?.detach().catch(report);
      }
    };
  }, []);

  const active = sessions.find((session) => session.key === activeKey) ?? null;
  const send = (operation: Promise<void> | undefined) => operation?.catch(fail);

  const handleSize = React.useCallback(
    (size: TerminalViewportSize) => {
      sizeRef.current = size;
      const connection = sessionsRef.current.find(
        (session) => session.key === activeKey,
      )?.connection;
      if (!connection) return;
      resizeChainRef.current = resizeChainRef.current
        .then(async () => {
          const viewport = await connection.resize(
            size.columns,
            size.rows,
            size.pixelWidth,
            size.pixelHeight,
          );
          await connection.viewportReady(viewport);
        })
        .catch(fail);
    },
    [activeKey, fail],
  );

  return (
    <TerminalSubstrate
      bracketedPaste={active?.frame?.bracketedPaste ?? false}
      channelName={channelName}
      enabled={available && Boolean(active)}
      focusReportingEnabled={active?.frame?.focusReporting ?? false}
      frame={active?.frame}
      sessionFrames={sessions.flatMap((session) =>
        session.frame ? [{ sessionId: session.key, frame: session.frame }] : [],
      )}
      onCloseSession={(key) => {
        setSessions((current) =>
          current.map((session) =>
            session.key === key ? { ...session, closing: true } : session,
          ),
        );
        const connection = sessionsRef.current.find(
          (session) => session.key === key,
        )?.connection;
        if (!connection) {
          removeSession(key);
          return;
        }
        void connection
          .close()
          .then(() => removeSession(key))
          .catch(fail);
      }}
      onFrameConsumed={(frame) => {
        const delivery = sessionsRef.current.find(
          (session) => session.delivery?.frame === frame,
        )?.delivery;
        if (!delivery) return;
        const { sequence, subscriptionId } = delivery.frame;
        const lastAcknowledged =
          acknowledgedSequenceRef.current.get(subscriptionId) ?? -1;
        if (sequence <= lastAcknowledged) return;
        acknowledgedSequenceRef.current.set(subscriptionId, sequence);
        delivery.acknowledge().catch((error) => {
          if (
            acknowledgedSequenceRef.current.get(subscriptionId) === sequence
          ) {
            acknowledgedSequenceRef.current.set(
              subscriptionId,
              lastAcknowledged,
            );
          }
          fail(error);
        });
      }}
      onInput={(text) => send(active?.connection?.input(text))}
      onNewSession={createSession}
      onScroll={(lines) => send(active?.connection?.scroll(lines))}
      onSelectSession={setActiveKey}
      onTerminalFocusChange={(focused) =>
        send(active?.connection?.focus(focused))
      }
      onViewportSize={handleSize}
      sessions={sessions.map((session) => ({
        active: session.key === activeKey,
        closing: session.closing,
        id: session.key,
        title: session.title,
      }))}
    />
  );
}
