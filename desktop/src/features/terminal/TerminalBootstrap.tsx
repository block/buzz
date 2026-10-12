import * as React from "react";
import { publishTerminalTabs } from "./terminalTabsBridge";
import { getWebTermInfo } from "@/features/bridge/webTermHub";
import { isTauri } from "@tauri-apps/api/core";
import { createFrameCoalescer, createInputCoalescer } from "./webTermCoalesce";
import { termProbe } from "@/platform/web/kbDebug/termProbe";

import {
  type TerminalDelivery,
  TerminalConnection,
  type TerminalConnectionApi,
  type TerminalFrameMessage,
  type TerminalMessage,
  getTerminalBackend,
  useTerminalBackend,
} from "./terminalClient";
import {
  TerminalSubstrate,
  type TerminalViewportSize,
} from "./TerminalSubstrate";
import {
  openTerminalPanel,
  setTerminalPanelMode,
  setTerminalSessionChannels,
  toggleTerminalPanel,
  useTerminalPanel,
} from "./terminalPanelStore";
import {
  clearTermSessionLive,
  consumeTermSessionOpenRequest,
  getTermSessionLiveKey,
  noteTermSessionLive,
  subscribeTermSessionOpenRequest,
  type TermSessionOpenRequest,
} from "@/features/term-session/lib/openRequestStore";
import { homeDir } from "@tauri-apps/api/path";
import {
  getTermPreferences,
  resolveTermCwdForAttach,
} from "./termPreferences.ts";
import {
  herdrAttachKey,
  herdrWorkspaceLabel,
  openInHerdr,
  shouldOpenInHerdr,
  type HerdrOpenResult,
} from "./herdrHost.ts";
import { toast } from "sonner";

type TerminalContext = {
  channelId: string;
  channelName: string;
  threadId: string | null;
  npub: string;
  relayUrl: string;
};

type Session = {
  key: string;
  connection: TerminalConnectionApi | null;
  delivery: TerminalDelivery | null;
  frame: TerminalFrameMessage | undefined;
  title: string;
  closing: boolean;
  context: TerminalContext;
  /** Buzz Term handoff card sid when this tab was opened from a term-session card. */
  termSessionSid?: string | null;
  pendingLaunchCommand?: string | null;
  extraEnv?: Record<string, string>;
  /**
   * When set, this tab is the in-app herdr-attach PTY for that session name
   * (blank string = herdr unnamed default). Reused across plain opens.
   */
  herdrAttachSession?: string | null;
};

type CreateSessionOptions = {
  /** New-tab menu "Terminal": a plain login shell even when the Session host is herdr (web: asks the desktop for a shell). */
  forceShell?: boolean;
  /** + menu "herdr": explicit choice — never fall back to a plain shell just because an attach tab exists. */
  forceHerdr?: boolean;
  /** Web: reattach this desktop terminal (survived a refresh / dropped link) instead of launching a new one. */
  reattach?: { id: string; title?: string | null };
  termSessionSid?: string | null;
  pendingLaunchCommand?: string | null;
  extraEnv?: Record<string, string>;
  /** Per-tab cwd from a Term Session handoff card; wins over Settings. */
  cwd?: string | null;
  contextOverride?: Partial<TerminalContext>;
  /** Spawn herdr TUI in-app instead of a login shell. */
  herdrAttach?: {
    bin: string;
    argv: string[];
    sessionKey: string;
  };
};

/** The desktop's remote listener allows 3 terminals (reattached/detached ones included). */
const REMOTE_TAB_CAP = 3;
const tabLimitToast = () => toast.message(`Tab limit reached (${REMOTE_TAB_CAP})`, { id: "buzz-term-tab-limit", duration: 2500 });
const isTooMany = (e: unknown) => (e as { code?: string } | null)?.code === "too_many_terminals" || /too_many_terminals/.test(String((e as Error)?.message ?? e));

const INITIAL_SIZE: TerminalViewportSize = {
  columns: 80,
  rows: 24,
  pixelWidth: 672,
  pixelHeight: 408,
};

/** Synthetic channel for left-nav / all-scope herdr when no channel is selected. */
const FALLBACK_TERM_CHANNEL_ID = "__buzz_term__";
const FALLBACK_TERM_CHANNEL_NAME = "Buzz Term";

function report(error: unknown) {
  console.error("terminal session failed", error);
}

/**
 * Web: sessions come from the member's desktop through a registered backend (the Buzz term page registers it) instead of the
 * Tauri runtime. The desktop's own Buzz Term settings apply there, so this device's local Session host / shell / cwd / scrollback
 * settings are ignored: it always asks for a plain launch and the desktop decides shell vs herdr.
 */
function isRemoteTerminal(): boolean {
  return !isTauri() && getTerminalBackend() !== null;
}
function effectiveSessionHost(): "buzz-term" | "herdr" {
  return isRemoteTerminal() ? "buzz-term" : getTermPreferences().sessionHost;
}
function effectiveOpenMode(): "docked" | "maximized" {
  return isRemoteTerminal() ? "maximized" : getTermPreferences().openMode;
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
          // Prefer display name from useTerminalContext; never stuff the raw
          // channel id into the name slot when a title is available.
          channelName: channelName?.trim() || "channel",
          threadId,
          npub,
          relayUrl,
        }
      : null;
  const contextRef = React.useRef<TerminalContext | null>(context);
  contextRef.current = context;
  // Identity alone (no channel) — enough for all-scope herdr attach.
  const identityRef = React.useRef<{ npub: string; relayUrl: string } | null>(
    null,
  );
  const remote = useTerminalBackend() !== null && !isTauri();
  identityRef.current =
    npub && relayUrl
      ? { npub, relayUrl }
      : remote
        ? { npub: npub ?? "web", relayUrl: relayUrl ?? "web" }
        : null;
  const mountedRef = React.useRef(true);
  const sizeRef = React.useRef(INITIAL_SIZE);
  const resizeChainRef = React.useRef(Promise.resolve());
  const connectionSizesRef = React.useRef(
    new WeakMap<TerminalConnectionApi, TerminalViewportSize>(),
  );
  const closedSessionKeysRef = React.useRef(new Set<string>());
  const [sessions, setSessions] = React.useState<Session[]>([]);
  const [activeKey, setActiveKey] = React.useState<string | null>(null);
  const [tauriAvailable, setAvailable] = React.useState(() => isTauri());
  // A registered remote backend means "available" from the start; a failed attach reports through the backend instead of
  // permanently switching Term off, so the page can offer Reconnect.
  const available = remote || tauriAvailable;
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
  const reattachInFlightRef = React.useRef(false);
  const reattachFailuresRef = React.useRef(0);
  const sessionsRef = React.useRef(sessions);
  sessionsRef.current = sessions;
  /** When a term-session handoff is creating the first tab, skip blank auto-create. */
  const skipAutoCreateRef = React.useRef(false);
  /** Dedup concurrent herdr ensure/attach opens so the panel is not left empty. */
  const herdrOpenInFlightRef = React.useRef(false);
  const createInAppSessionRef = React.useRef<
    (options?: CreateSessionOptions) => void
  >(() => {});

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

  // Last successful GUI / tab spawn context. Lets maximized Term (and New tab)
  // keep working after navigating away from a channel without inventing one.
  const lastSpawnContextRef = React.useRef<TerminalContext | null>(null);
  if (context) lastSpawnContextRef.current = context;

  React.useEffect(() => {
    const toggle = (event: KeyboardEvent) => {
      const canSpawn =
        Boolean(contextRef.current) || Boolean(lastSpawnContextRef.current);
      const hasSessions = sessionsRef.current.length > 0;
      if (
        (!canSpawn && !hasSessions) ||
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
    if (isRemoteTerminal()) {
      getTerminalBackend()?.onError?.(error);
      setTerminalPanelMode("closed");
      return;
    }
    setAvailable(false);
  }, []);

  const removeSession = React.useCallback((key: string) => {
    const closing = sessionsRef.current.find((session) => session.key === key);
    if (closing?.termSessionSid) {
      clearTermSessionLive(closing.termSessionSid);
    }
    setSessions((current) => current.filter((session) => session.key !== key));
    setActiveKey((current) => {
      if (current !== key) return current;
      const remaining = sessionsRef.current.filter(
        (session) => session.key !== key,
      );
      return remaining.at(-1)?.key ?? null;
    });
  }, []);

  const findHerdrAttachSession = React.useCallback((sessionKey: string) => {
    return sessionsRef.current.find(
      (session) =>
        !session.closing && session.herdrAttachSession === sessionKey,
    );
  }, []);

  /** Prefer live channel context; else last spawn; else synthetic for herdr all-scope. */
  const resolveSpawnContext = React.useCallback((): TerminalContext | null => {
    const live =
      contextRef.current ??
      sessionsRef.current.at(-1)?.context ??
      lastSpawnContextRef.current;
    if (live) return live;
    const identity = identityRef.current;
    if (!identity) return null;
    return {
      channelId: FALLBACK_TERM_CHANNEL_ID,
      channelName: FALLBACK_TERM_CHANNEL_NAME,
      threadId: null,
      npub: identity.npub,
      relayUrl: identity.relayUrl,
    };
  }, []);

  const openOrReuseHerdrAttach = React.useCallback(
    (
      result: Extract<HerdrOpenResult, { ok: true }>,
      options?: CreateSessionOptions,
    ) => {
      const sessionKey = herdrAttachKey(result.sessionName);
      const existing = findHerdrAttachSession(sessionKey);
      const mode = panel.mode === "closed" ? effectiveOpenMode() : panel.mode;
      openTerminalPanel(mode, panel.tabScope);
      if (existing) {
        setActiveKey(existing.key);
        return;
      }
      createInAppSessionRef.current({
        ...options,
        // Handoff command runs inside the herdr workspace pane (send-text),
        // not typed into the attach TUI PTY.
        pendingLaunchCommand: null,
        herdrAttach: {
          bin: result.herdrBin,
          argv: result.attachArgv,
          sessionKey,
        },
      });
    },
    [findHerdrAttachSession, panel.mode, panel.tabScope],
  );

  const createSession = React.useCallback(
    (options?: CreateSessionOptions) => {
      const baseContext = resolveSpawnContext();
      if (!available || !baseContext) return;
      if (isRemoteTerminal() && !options?.reattach && sessionsRef.current.length >= REMOTE_TAB_CAP) {
        tabLimitToast();
        return;
      }
      if (options?.forceShell) {
        createInAppSessionRef.current(options);
        return;
      }
      // Web, no tabs yet (page load, refresh, Reconnect): bring back the desktop terminals that are still running first.
      const backend = getTerminalBackend();
      if (isRemoteTerminal() && sessionsRef.current.length === 0 && !options?.reattach && backend?.listLive && !options?.pendingLaunchCommand) {
        if (reattachInFlightRef.current) return;
        reattachInFlightRef.current = true;
        void backend
          .listLive()
          .then((live) => {
            if (!mountedRef.current) return;
            if (live.length === 0 || sessionsRef.current.length > 0) {
              if (sessionsRef.current.length === 0) createInAppSessionRef.current(options);
              return;
            }
            for (const t of live) createInAppSessionRef.current({ reattach: { id: t.id, title: t.title } });
          })
          // couldn't even list (signer declined, link down): never launch blind into a full cap — report it
          .catch((error) => fail(error))
          .finally(() => {
            reattachInFlightRef.current = false;
          });
        return;
      }

      // Fast path: Buzz Term host — create the in-app PTY immediately (no async
      // herdr probe) so the panel never sits empty.
      if (effectiveSessionHost() !== "herdr") {
        createInAppSessionRef.current(options);
        return;
      }

      // herdr host: attach in-app (never Terminal.app). Plain open does NOT
      // create a workspace — reuse last focused / first space.
      const prefs = getTermPreferences();
      const sessionKey = herdrAttachKey(prefs.sessionName);
      const existing = findHerdrAttachSession(sessionKey);
      const reopenMode = panel.mode === "closed" ? effectiveOpenMode() : panel.mode;
      const scope = panel.tabScope;
      // Open chrome immediately so left-nav / ⌘J never flash an empty panel
      // while the herdr probe runs.
      openTerminalPanel(reopenMode, scope);
      if (existing && !options?.forceHerdr) {
        // Attach tab already live — New tab (+) opens another in-app shell
        // (plain open never createWorkspace).
        createInAppSessionRef.current(options);
        return;
      }
      if (herdrOpenInFlightRef.current) return;
      herdrOpenInFlightRef.current = true;
      void (async () => {
        try {
          if (!(await shouldOpenInHerdr())) {
            createInAppSessionRef.current(options);
            return;
          }
          // Explicit herdr pick with an attach tab already open: add a new herdr workspace and focus that tab.
          const result = await openInHerdr({ createWorkspace: Boolean(existing && options?.forceHerdr) });
          if (result.ok && existing && options?.forceHerdr) {
            // + → herdr with a herdr tab already open: a NEW tab with its own attach client (unique tab key, so the
            // attach-by-session-name dedupe below can't fold it into the first tab). It shows the workspace just created.
            const mode = panel.mode === "closed" ? effectiveOpenMode() : panel.mode;
            openTerminalPanel(mode, panel.tabScope);
            createInAppSessionRef.current({
              ...options,
              pendingLaunchCommand: null,
              herdrAttach: {
                bin: result.herdrBin,
                argv: result.attachArgv,
                sessionKey: `${herdrAttachKey(result.sessionName)}#${result.workspaceId ?? crypto.randomUUID()}`,
              },
            });
            return;
          }
          if (result.ok) {
            // Re-check reuse: a parallel open may have created the attach tab.
            const again = findHerdrAttachSession(
              herdrAttachKey(result.sessionName),
            );
            if (again) {
              setActiveKey(again.key);
              return;
            }
            openOrReuseHerdrAttach(result, options);
            return;
          }
          toast.message("herdr unavailable — using Buzz Term", {
            description: result.reason,
          });
          createInAppSessionRef.current(options);
        } catch (error) {
          toast.message("herdr unavailable — using Buzz Term", {
            description: error instanceof Error ? error.message : String(error),
          });
          createInAppSessionRef.current(options);
        } finally {
          herdrOpenInFlightRef.current = false;
        }
      })();
    },
    [
      available,
      findHerdrAttachSession,
      openOrReuseHerdrAttach,
      panel.mode,
      panel.tabScope,
      resolveSpawnContext,
    ],
  );

  const createInAppSession = React.useCallback(
    (options?: CreateSessionOptions) => {
      const baseContext = resolveSpawnContext();
      if (!available || !baseContext) return;
      const spawnContext: TerminalContext = {
        ...baseContext,
        ...options?.contextOverride,
        channelId: options?.contextOverride?.channelId ?? baseContext.channelId,
        channelName:
          options?.contextOverride?.channelName ?? baseContext.channelName,
        threadId:
          options?.contextOverride?.threadId !== undefined
            ? options.contextOverride.threadId
            : baseContext.threadId,
        npub: options?.contextOverride?.npub ?? baseContext.npub,
        relayUrl: options?.contextOverride?.relayUrl ?? baseContext.relayUrl,
      };
      lastSpawnContextRef.current = spawnContext;
      const key = crypto.randomUUID();
      const herdrAttach = options?.herdrAttach;
      const initial: Session = {
        key,
        connection: null,
        delivery: null,
        frame: undefined,
        title: herdrAttach ? "herdr" : options?.reattach?.title || "SHELL",
        closing: false,
        context: spawnContext,
        termSessionSid: options?.termSessionSid ?? null,
        pendingLaunchCommand: options?.pendingLaunchCommand ?? null,
        extraEnv: options?.extraEnv,
        herdrAttachSession: herdrAttach?.sessionKey ?? null,
      };
      setSessions((current) => [...current, initial]);
      setActiveKey(key);
      if (options?.termSessionSid) {
        noteTermSessionLive(options.termSessionSid, key);
      }

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
      const applyFrame = (delivery: TerminalDelivery) => {
        update((session) => ({ ...session, delivery, frame: delivery.frame }));
      };
      // Web: merge frames that outpace the screen and render once per animation frame (desktop unchanged).
      // Web: the desktop keeps one frame in flight and sends the next only after our ack. Acking after React has
      // painted put render time inside every keystroke's round trip; ack on arrival instead so the next frame is on
      // the wire while this one paints, and merge any that land before the next paint (desktop unchanged).
      const webFrames = isTauri() ? null : createFrameCoalescer(applyFrame);
      const onFrame = !webFrames
        ? applyFrame
        : (delivery: TerminalDelivery) => {
            termProbe.frameArrived();
            const { sequence, subscriptionId } = delivery.frame;
            if (sequence > (acknowledgedSequenceRef.current.get(subscriptionId) ?? -1)) {
              acknowledgedSequenceRef.current.set(subscriptionId, sequence);
              void delivery.acknowledge().catch(() => {});
            }
            webFrames.push(delivery);
          };

      const size = sizeRef.current;
      void (async () => {
        const prefs = getTermPreferences();
        // Remote: the desktop picks the directory (its own setting); a path from this device or a card means nothing there.
        const cwd = remote
          ? undefined
          : await resolveTermCwdForAttach({
              preference: prefs.cwd,
              handoffCwd: options?.cwd,
              homeDir,
            });
        return TerminalConnection.attach(
          {
            ...spawnContext,
            threadId: spawnContext.threadId ?? undefined,
            ...size,
            ...(options?.extraEnv && !remote ? { extraEnv: options.extraEnv } : {}),
            ...(cwd ? { cwd } : {}),
            ...(prefs.shell && !remote ? { shell: prefs.shell } : {}),
            // a handoff launch line is typed into a shell, so never into the desktop's herdr UI
            ...(remote && (options?.pendingLaunchCommand || options?.forceShell) ? { remoteHost: "shell" as const } : {}),
            ...(remote && options?.reattach ? { reattachId: options.reattach.id } : {}),
            ...(herdrAttach
              ? { program: herdrAttach.bin, args: herdrAttach.argv }
              : {}),
            scrollback: prefs.scrollback,
          },
          onMessage,
          onFrame,
        );
      })()
        .then((connection) => {
          if (!mountedRef.current) return connection.detach();
          if (closedSessionKeysRef.current.delete(key))
            return connection.close();
          connectionSizesRef.current.set(connection, size);
          update((session) => ({ ...session, connection }));
          // Never type handoff into the herdr-attach TUI; card Open uses
          // herdr pane send-text instead.
          const launch = herdrAttach
            ? null
            : options?.pendingLaunchCommand?.trim();
          if (launch) {
            // Give the shell a moment to paint the prompt before typing.
            window.setTimeout(() => {
              void connection.input(`${launch}\n`).catch(fail);
              update((session) => ({
                ...session,
                pendingLaunchCommand: null,
              }));
            }, 350);
          }
          // a reattached terminal still has the size of the device that last drove it
          if (sizeRef.current !== size || options?.reattach) {
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
          // Desktop cap hit (e.g. detached sessions count): keep the panel and the other tabs, just say so.
          // Reattach keeps failing (not a declined signature): close the orphans so the next open starts clean.
          if (remote && options?.reattach && !/declin|reject|denied/i.test(String((error as Error)?.message ?? error))) {
            reattachFailuresRef.current += 1;
            if (reattachFailuresRef.current >= 2) {
              reattachFailuresRef.current = 0;
              void getTerminalBackend()?.closeAllLive?.().catch(() => 0);
            }
          }
          if (remote && isTooMany(error)) {
            tabLimitToast();
            // the only tab was refused: fall through to fail() → the page explains it and offers "Close old sessions"
            if (sessionsRef.current.some((x) => x.key !== key)) return;
          }
          // herdr program spawn failed — fall back to a login shell instead of
          // permanently disabling Term (available=false) and leaving chrome blank.
          if (herdrAttach) {
            toast.message("herdr attach failed — using Buzz Term", {
              description:
                error instanceof Error ? error.message : String(error),
            });
            createInAppSessionRef.current({
              ...options,
              herdrAttach: undefined,
              pendingLaunchCommand: options?.pendingLaunchCommand ?? null,
            });
            return;
          }
          fail(error);
        });
    },
    [available, fail, remote, removeSession, resolveSpawnContext],
  );
  createInAppSessionRef.current = createInAppSession;

  const fulfillTermSessionOpen = React.useCallback(
    (request: TermSessionOpenRequest) => {
      const liveKey = getTermSessionLiveKey(request.sid);
      const live = liveKey
        ? sessionsRef.current.find((session) => session.key === liveKey)
        : null;
      if (live && !live.closing) {
        openTerminalPanel(effectiveOpenMode(), remote ? "all" : "channel");
        setActiveKey(live.key);
        return;
      }

      void (async () => {
        // Card Open + herdr host: create ONE focused workspace, send handoff
        // command into its root pane, then show/reuse the in-app herdr attach.
        if (
          !remote &&
          request.launchCommand.trim() &&
          (await shouldOpenInHerdr())
        ) {
          const result = await openInHerdr({
            createWorkspace: true,
            cwd: request.cwd,
            command: request.launchCommand,
            label: herdrWorkspaceLabel(
              request.channelName,
              request.threadId,
            ),
            env: request.extraEnv,
          });
          if (result.ok) {
            skipAutoCreateRef.current = true;
            openOrReuseHerdrAttach(result, {
              termSessionSid: request.sid,
              extraEnv: request.extraEnv,
              cwd: request.cwd,
              contextOverride: {
                channelId: request.channelId,
                channelName: request.channelName,
                threadId: request.threadId,
              },
            });
            toast.success("Opened term session in herdr");
            window.setTimeout(() => {
              skipAutoCreateRef.current = false;
            }, 0);
            return;
          }
          toast.message("herdr unavailable — using Buzz Term", {
            description: result.reason,
          });
        }

        if (
          !remote &&
          !contextRef.current &&
          !sessionsRef.current.at(-1)?.context &&
          !lastSpawnContextRef.current
        ) {
          openTerminalPanel(effectiveOpenMode(), remote ? "all" : "channel");
          return;
        }
        if (!request.launchCommand.trim()) {
          openTerminalPanel(effectiveOpenMode(), remote ? "all" : "channel");
          return;
        }
        if (!available) {
          openTerminalPanel(effectiveOpenMode(), remote ? "all" : "channel");
          return;
        }
        skipAutoCreateRef.current = true;
        openTerminalPanel(effectiveOpenMode(), remote ? "all" : "channel");
        createInAppSession({
          termSessionSid: request.sid,
          pendingLaunchCommand: request.launchCommand,
          extraEnv: request.extraEnv,
          cwd: request.cwd,
          contextOverride: {
            channelId: request.channelId,
            channelName: request.channelName,
            threadId: request.threadId,
          },
        });
        window.setTimeout(() => {
          skipAutoCreateRef.current = false;
        }, 0);
      })();
    },
    [available, createInAppSession, openOrReuseHerdrAttach, remote],
  );

  React.useEffect(() => {
    const onRequest = () => {
      const request = consumeTermSessionOpenRequest();
      if (request) fulfillTermSessionOpen(request);
    };
    return subscribeTermSessionOpenRequest(onRequest);
  }, [fulfillTermSessionOpen]);

  React.useEffect(() => {
    setTerminalSessionChannels(
      sessions.map((session) => session.context.channelId),
    );
  }, [sessions]);

  const contextChannelId = context?.channelId ?? null;
  const tabScope = panel.tabScope;

  // Visible tabs depend on how Term was opened:
  // - channel: only sessions for the active channel (in-channel / ⌘J).
  // - all: every live session (left-nav Buzz Term), with channel provenance.
  // Herdr-attach tabs are session-wide (not per-channel) and always visible
  // so plain reopen reuses the same in-app PTY instead of spawning another.
  const visibleSessions = React.useMemo(() => {
    if (tabScope === "all") return sessions;
    if (!contextChannelId) {
      return sessions.filter((session) => session.herdrAttachSession != null);
    }
    return sessions.filter(
      (session) =>
        session.context.channelId === contextChannelId ||
        session.herdrAttachSession != null,
    );
  }, [contextChannelId, sessions, tabScope]);

  // Hard guarantee: whenever the panel is open and the visible strip for the
  // current tabScope is empty, create an in-app session within one tick —
  // unless an explicit term-session handoff is in flight (skipAutoCreateRef).
  // herdr host goes through createSession (ensure server + in-app attach).
  React.useEffect(() => {
    if (panel.mode === "closed" || !available) return;

    if (visibleSessions.length > 0) {
      if (!visibleSessions.some((session) => session.key === activeKey)) {
        setActiveKey(visibleSessions.at(-1)?.key ?? null);
      }
      return;
    }

    // channel scope needs an active channel.
    // all scope: reuse last spawn, or (herdr host) identity-only synthetic
    // context so cold left-nav still attaches instead of sitting blank.
    // buzz-term host never invents a channel when none was selected.
    if (tabScope === "channel") {
      if (!context) return;
    } else {
      const canSpawn =
        Boolean(contextRef.current) ||
        Boolean(lastSpawnContextRef.current) ||
        remote ||
        (effectiveSessionHost() === "herdr" && Boolean(identityRef.current));
      if (!canSpawn) return;
    }

    const spawnEmpty = () => {
      // Web goes through createSession too: it lists the desktop's still-running terminals and reattaches them before
      // ever launching a new one (a direct create here used to skip that and hit the 3-terminal cap after a refresh).
      if (remote || effectiveSessionHost() === "herdr") {
        createSession();
      } else {
        // Buzz Term: attach immediately — no async host probe.
        createInAppSessionRef.current();
      }
    };

    // Handoff sets skip then createInAppSession before the effect; do not race.
    // If skip is still true with an empty strip (stuck after herdr/handoff),
    // clear on the next tick and create so chrome is never left blank.
    if (skipAutoCreateRef.current) {
      const timer = window.setTimeout(() => {
        skipAutoCreateRef.current = false;
        const live = sessionsRef.current;
        // Herdr-attach tabs are visible across channel scope — count them so
        // we do not spawn a duplicate when the strip is already covered.
        const stillEmpty =
          tabScope === "all"
            ? live.length === 0
            : !live.some(
                (session) =>
                  session.herdrAttachSession != null ||
                  session.context.channelId === contextRef.current?.channelId,
              );
        if (!stillEmpty) return;
        spawnEmpty();
      }, 0);
      return () => window.clearTimeout(timer);
    }

    spawnEmpty();
  }, [
    activeKey,
    available,
    context,
    createSession,
    panel.mode,
    remote,
    tabScope,
    visibleSessions,
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
    visibleSessions.find((session) => session.key === activeKey) ??
    visibleSessions.at(-1) ??
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

  const send = (operation: Promise<unknown> | undefined) => {
    void operation?.catch(fail);
  };
  // Web: first key goes out immediately; a key-repeat burst rides one message (desktop unchanged).
  const activeConnectionRef = React.useRef<TerminalConnectionApi | null | undefined>(null);
  activeConnectionRef.current = active?.connection;
  const sendTermInput = React.useCallback((text: string) => {
    termProbe.inputSent();
    void activeConnectionRef.current?.input(text)?.catch(fail);
  }, [fail]);
  const webInput = React.useMemo(() => createInputCoalescer((text) => sendTermInput(text)), [sendTermInput]);
  const onTermInput = isTauri() ? sendTermInput : webInput;
  const handleSplashStarted = React.useCallback(() => {
    setSplashPending(false);
  }, []);

  const webResizeTimerRef = React.useRef<number | undefined>(undefined);
  const handleSize = React.useCallback(
    (size: TerminalViewportSize) => {
      sizeRef.current = size;
      const connection = sessionsRef.current.find(
        (session) => session.key === activeKey,
      )?.connection;
      if (!connection) return;
      if (!isTauri()) {
        // Web: the keyboard animating, a rotation or a window drag reports many sizes in a row, and each resize makes
        // the remote app (e.g. a TUI) clear and redraw its whole screen, header/logo included. Send only the size it
        // settles on.
        window.clearTimeout(webResizeTimerRef.current);
        webResizeTimerRef.current = window.setTimeout(() => {
          const latest = sizeRef.current;
          const sent = connectionSizesRef.current.get(connection);
          if (!latest || (sent && sent.columns === latest.columns && sent.rows === latest.rows)) return;
          resizeChainRef.current = resizeChainRef.current
            .then(async () => {
              const viewport = await connection.resize(latest.columns, latest.rows, latest.pixelWidth, latest.pixelHeight);
              connectionSizesRef.current.set(connection, latest);
              await connection.viewportReady(viewport);
            })
            .catch(fail);
        }, 180);
        return;
      }
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

  const closeSession = (key: string) => {
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
  };
  const closeSessionRef = React.useRef(closeSession);
  closeSessionRef.current = closeSession;
  // Mirror the tab list for the phone tab switcher (same select/close handlers as the strip).
  const tabMirror = sessions
    .filter((session) => !session.closing)
    .map((session) => `${session.key}\u0000${session.title}\u0000${session.context.channelName ?? ""}`)
    .join("\u0001");
  React.useEffect(() => {
    publishTerminalTabs({
      tabs: sessionsRef.current
        .filter((session) => !session.closing)
        .map((session) => ({
          active: session.key === activeKey,
          channelName: session.context.channelName,
          id: session.key,
          title: session.title,
        })),
      select: setActiveKey,
      close: (key) => closeSessionRef.current(key),
    });
  }, [tabMirror, activeKey]);
  React.useEffect(() => () => publishTerminalTabs(null), []);

  if (!panelMounted) return null;

  return (
    <TerminalSubstrate
      bracketedPaste={active?.frame?.bracketedPaste ?? false}
      channelName={active?.context.channelName ?? channelName}
      enabled={
        available &&
        (tabScope === "all"
          ? sessions.length > 0 ||
            Boolean(context) ||
            Boolean(lastSpawnContextRef.current) ||
            remote ||
            (effectiveSessionHost() === "herdr" &&
              Boolean(identityRef.current))
          : Boolean(context))
      }
      tabScope={tabScope}
      mode={renderedMode}
      visible={panelVisible}
      onHide={() => setTerminalPanelMode("closed")}
      onModeChange={setTerminalPanelMode}
      onToggle={toggleTerminalPanel}
      focusReportingEnabled={active?.frame?.focusReporting ?? false}
      mouseReportingEnabled={active?.frame?.mouseReporting ?? false}
      frame={active?.frame}
      viewportReportingEnabled={viewportReportingEnabled}
      showSplash={splashPending && Boolean(active?.frame)}
      onSplashStarted={handleSplashStarted}
      sessionFrames={visibleSessions.flatMap((session) =>
        session.frame ? [{ sessionId: session.key, frame: session.frame }] : [],
      )}
      onCloseSession={closeSession}
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
      onInput={onTermInput}
      onNewSession={(kind) => createSession(kind === "shell" ? { forceShell: true } : kind === "herdr" ? { forceHerdr: true } : undefined)}
      canAddTab={!isRemoteTerminal() || sessions.length < REMOTE_TAB_CAP}
      herdrNewTab={
        isRemoteTerminal() ? getWebTermInfo()?.termHost === "herdr" : getTermPreferences().sessionHost === "herdr"
      }
      onScroll={(lines) => send(active?.connection?.scroll(lines))}
      onMouse={(event) => send(active?.connection?.mouse(event))}
      onSelectSession={setActiveKey}
      onTerminalFocusChange={(focused) =>
        send(active?.connection?.focus(focused))
      }
      onViewportSize={handleSize}
      sessions={visibleSessions
        .filter((session) => !session.closing)
        .map((session) => ({
          active: session.key === activeKey,
          channelName: session.context.channelName,
          closing: session.closing,
          id: session.key,
          title: session.title,
        }))}
    />
  );
}
