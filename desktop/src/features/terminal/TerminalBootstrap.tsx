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
import {
  setTerminalPanelMode,
  setTerminalSessionChannels,
  toggleTerminalPanel,
  useTerminalPanel,
} from "./terminalPanelStore";

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
  context: TerminalContext;
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
  const connectionSizesRef = React.useRef(
    new WeakMap<TerminalConnection, TerminalViewportSize>(),
  );
  const closedSessionKeysRef = React.useRef(new Set<string>());
  const [sessions, setSessions] = React.useState<Session[]>([]);
  const [activeKey, setActiveKey] = React.useState<string | null>(null);
  const [available, setAvailable] = React.useState(() => isTauri());
  const panel = useTerminalPanel();
  const [renderedMode, setRenderedMode] = React.useState<
    "docked" | "maximized"
  >(panel.mode === "maximized" ? "maximized" : "docked");
  const [panelVisible, setPanelVisible] = React.useState(
    panel.mode !== "closed",
  );
  const [panelMounted, setPanelMounted] = React.useState(
    panel.mode !== "closed",
  );
  const [splashPending, setSplashPending] = React.useState(true);
  const [viewportReportingEnabled, setViewportReportingEnabled] =
    React.useState(panel.mode !== "closed");
  const previousPanelModeRef = React.useRef(panel.mode);
  const acknowledgedSequenceRef = React.useRef(new Map<string, number>());
  const sessionsRef = React.useRef(sessions);
  sessionsRef.current = sessions;

  React.useEffect(() => {
    const previousMode = previousPanelModeRef.current;
    if (previousMode === panel.mode) return;
    previousPanelModeRef.current = panel.mode;
    setViewportReportingEnabled(false);

    let firstFrame = 0;
    let secondFrame = 0;
    let timeout = 0;
    if (panel.mode !== "closed") {
      setRenderedMode(panel.mode);
      setPanelMounted(true);
      if (previousMode === "closed") {
        // Give the collapsed substrate a painted frame before expanding it.
        // A single rAF can still be batched into the mount commit by React.
        setPanelVisible(false);
        firstFrame = window.requestAnimationFrame(() => {
          secondFrame = window.requestAnimationFrame(() =>
            setPanelVisible(true),
          );
        });
      } else {
        setPanelVisible(true);
      }
      // Resizing the PTY through every animation frame causes shell reflow and
      // leaves transient filler rows in the scrollback. Publish only the final
      // settled viewport.
      timeout = window.setTimeout(() => setViewportReportingEnabled(true), 200);
    } else {
      setPanelVisible(false);
      timeout = window.setTimeout(() => setPanelMounted(false), 180);
    }
    return () => {
      window.cancelAnimationFrame(firstFrame);
      window.cancelAnimationFrame(secondFrame);
      window.clearTimeout(timeout);
    };
  }, [panel.mode]);

  React.useEffect(() => {
    if (!context && panel.mode !== "closed") setTerminalPanelMode("closed");
  }, [context, panel.mode]);

  React.useEffect(() => {
    const toggle = (event: KeyboardEvent) => {
      if (
        !context ||
        panel.mode !== "closed" ||
        event.code !== "KeyJ" ||
        (!event.metaKey && !event.ctrlKey) ||
        event.altKey ||
        event.shiftKey ||
        event.isComposing
      )
        return;
      event.preventDefault();
      event.stopImmediatePropagation();
      if (event.type === "keyup") toggleTerminalPanel();
    };
    window.addEventListener("keydown", toggle, true);
    window.addEventListener("keyup", toggle, true);
    return () => {
      window.removeEventListener("keydown", toggle, true);
      window.removeEventListener("keyup", toggle, true);
    };
  }, [context, panel.mode]);

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
      context: spawnContext,
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
        if (closedSessionKeysRef.current.delete(key)) return connection.close();
        connectionSizesRef.current.set(connection, size);
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
              connectionSizesRef.current.set(connection, currentSize);
              await connection.viewportReady(viewport);
            })
            .catch(fail);
        }
      })
      .catch((error) => {
        if (closedSessionKeysRef.current.delete(key)) return;
        removeSession(key);
        fail(error);
      });
  }, [available, fail, removeSession]);

  const contextChannelId = context?.channelId ?? null;
  const channelSessions = React.useMemo(
    () =>
      contextChannelId
        ? sessions.filter(
            (session) => session.context.channelId === contextChannelId,
          )
        : [],
    [contextChannelId, sessions],
  );

  React.useEffect(() => {
    setTerminalSessionChannels(
      sessions.map((session) => session.context.channelId),
    );
  }, [sessions]);

  React.useEffect(() => {
    if (panel.mode === "closed" || !available || !context) return;
    if (channelSessions.length === 0) createSession();
    else if (!channelSessions.some((session) => session.key === activeKey))
      setActiveKey(channelSessions.at(-1)?.key ?? null);
  }, [
    activeKey,
    available,
    channelSessions,
    context,
    createSession,
    panel.mode,
  ]);

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      for (const session of sessionsRef.current) {
        void session.connection?.detach().catch(report);
      }
    };
  }, []);

  const active =
    channelSessions.find((session) => session.key === activeKey) ??
    channelSessions.at(-1) ??
    null;

  React.useEffect(() => {
    const connection = active?.connection;
    if (!connection) return;
    const size = sizeRef.current;
    if (connectionSizesRef.current.get(connection) === size) return;
    resizeChainRef.current = resizeChainRef.current
      .then(async () => {
        const viewport = await connection.resize(
          size.columns,
          size.rows,
          size.pixelWidth,
          size.pixelHeight,
        );
        connectionSizesRef.current.set(connection, size);
        await connection.viewportReady(viewport);
      })
      .catch(fail);
  }, [active?.connection, fail]);

  const send = (operation: Promise<void> | undefined) => operation?.catch(fail);
  const handleSplashStarted = React.useCallback(() => {
    setSplashPending(false);
  }, []);

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
          connectionSizesRef.current.set(connection, size);
          await connection.viewportReady(viewport);
        })
        .catch(fail);
    },
    [activeKey, fail],
  );

  if (!panelMounted) return null;

  return (
    <TerminalSubstrate
      bracketedPaste={active?.frame?.bracketedPaste ?? false}
      channelName={active?.context.channelName ?? channelName}
      enabled={available && Boolean(context)}
      mode={renderedMode}
      visible={panelVisible}
      onHide={() => setTerminalPanelMode("closed")}
      onModeChange={setTerminalPanelMode}
      onToggle={toggleTerminalPanel}
      focusReportingEnabled={active?.frame?.focusReporting ?? false}
      frame={active?.frame}
      viewportReportingEnabled={viewportReportingEnabled}
      showSplash={splashPending && Boolean(active?.frame)}
      onSplashStarted={handleSplashStarted}
      sessionFrames={channelSessions.flatMap((session) =>
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
          closedSessionKeysRef.current.add(key);
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
      sessions={channelSessions
        .filter((session) => !session.closing)
        .map((session) => ({
          active: session.key === activeKey,
          closing: session.closing,
          id: session.key,
          title: session.title,
        }))}
    />
  );
}
