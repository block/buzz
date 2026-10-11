// Mounted once in the desktop AppShell (no-op in the browser). Feeds the bridge publisher from existing agent state.
import { isTauri } from "@tauri-apps/api/core";
import * as React from "react";

import { useActiveAgentTurnsByChannel } from "@/features/agents/activeAgentTurnsStore";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  getAgentTranscript,
  subscribeAgentObserverStore,
} from "@/features/agents/observerRelayStore";
import { relayClient } from "@/shared/api/relayClient";
import { nip44EncryptToSelf, signRelayEvent } from "@/shared/api/tauri";
import { sendChannelMessage } from "@/shared/api/tauriMessages";
import {
  startManagedAgent,
  stopManagedAgent,
} from "@/shared/api/tauriManagedAgents";
import type { RelayEvent } from "@/shared/api/types";
import { isAgentManagementAllowed, remoteInfoForHeartbeat, subscribeRemoteAccess } from "./remoteAccessStore";
import { makeBrowserDeps, playgroundBrowserHost, remoteBrowserManager, snapshotBrowserSessions } from "./browserHost";
import { BridgeCommandHandler, makeSeenSet } from "./commandHandler";
import { BRIDGE_KIND, BRIDGE_TAG, CMD_MAX_TTL_S, nowS } from "./protocol";
import { BridgePublisher } from "./publisher";
import { buildAgentsSnapshot } from "./snapshot";
import { entriesFromItems } from "./transcript";

const DEVICE_KEY = "buzz-bridge-device-id";

function deviceId(): string {
  try {
    let id = localStorage.getItem(DEVICE_KEY);
    if (!id) {
      id = crypto.randomUUID();
      localStorage.setItem(DEVICE_KEY, id);
    }
    return id;
  } catch {
    return "ephemeral";
  }
}

export function useBridgePublisher(
  pubkey: string | undefined,
  relayUrl: string | undefined,
  channelName: (channelId: string) => string | undefined,
): void {
  const enabled = isTauri() && !!pubkey && !!relayUrl;
  const agentsQuery = useManagedAgentsQuery({ enabled });
  const channelTurns = useActiveAgentTurnsByChannel();
  const publisherRef = React.useRef<BridgePublisher | null>(null);
  const nameRef = React.useRef(channelName);
  nameRef.current = channelName;
  const agentNamesRef = React.useRef<(pubkey: string) => string | undefined>(() => undefined);
  agentNamesRef.current = (pk) => agentsQuery.data?.find((a) => a.pubkey === pk)?.name;

  // biome-ignore lint/correctness/useExhaustiveDependencies: pubkey/relayUrl identify the session; a change must restart the publisher (new key wrap).
  React.useEffect(() => {
    if (!enabled) return;
    const p = new BridgePublisher(
      {
        encryptToSelf: nip44EncryptToSelf,
        signEvent: (e) => signRelayEvent(e),
        publish: async (signed) => {
          await relayClient.publishEvent(
            signed as RelayEvent,
            "Timed out publishing bridge event.",
            "Failed to publish bridge event.",
          );
        },
      },
      {
        deviceId: deviceId(),
        platform: "desktop",
        caps: [
          "agents.snapshot",
          "transcript.chunk",
          "agent.start",
          "agent.stop",
          "agent.message",
          "browser.open",
          "browser.close",
          "browser.screenshot",
          "browser.agent",
        ],
        remote: remoteInfoForHeartbeat,
        onError: (e) => console.warn("[bridge] publish failed:", e),
      },
    );
    publisherRef.current = p;
    p.start();
    // Tell the web right away when remote access / terminals flip (not just at the next heartbeat).
    let lastRemote = JSON.stringify(remoteInfoForHeartbeat() ?? null);
    const unsubRemote = subscribeRemoteAccess(() => {
      const now = JSON.stringify(remoteInfoForHeartbeat() ?? null);
      if (now !== lastRemote) {
        lastRemote = now;
        void p.sendHeartbeat();
      }
    });
    // Browser sync: publish the desktop's whole Browsers page now and whenever it (or an agent grant, or liveness) changes.
    let lastSessions = "";
    let syncTimer: ReturnType<typeof setTimeout> | null = null;
    let syncing = false;
    const syncBrowsers = async (force = false) => {
      if (syncing) return;
      syncing = true;
      try {
        const live = new Set(playgroundBrowserHost.rows().map((r) => r.sid));
        for (const id of remoteBrowserManager.ids()) if (!live.has(id)) remoteBrowserManager.remove(id);
        const list = await snapshotBrowserSessions(playgroundBrowserHost, (pk) => agentNamesRef.current(pk), nowS());
        const key = JSON.stringify(list.map(({ openedAt: _o, ...rest }) => rest));
        if (force || key !== lastSessions) {
          lastSessions = key;
          await p.publishBrowserSessions(list);
        }
      } catch (e) {
        console.warn("[bridge] browser sync failed:", e);
      } finally {
        syncing = false;
      }
    };
    const scheduleSync = () => {
      if (syncTimer) clearTimeout(syncTimer);
      syncTimer = setTimeout(() => void syncBrowsers(), 600);
    };
    void syncBrowsers(true);
    const unsubBrowsers = playgroundBrowserHost.subscribe(scheduleSync);
    const unsubGrants = playgroundBrowserHost.subscribeGrants(scheduleSync);
    const syncInterval = setInterval(() => void syncBrowsers(), 20_000); // liveness (Cold/Live) changes without a store event
    // Commands from the member's own browser (validated in BridgeCommandHandler; see docs/pwa-bridge-protocol.md §6).
    const SEEN_KEY = "buzz-bridge-cmd-seen";
    const handler = new BridgeCommandHandler({
      memberPubkey: (pubkey ?? "").toLowerCase(),
      sessionKey: () => ({ kid: p.kid, key: p.sessionKey }),
      agents: () => p.agents(),
      seen: makeSeenSet(
        () => {
          try {
            return JSON.parse(localStorage.getItem(SEEN_KEY) ?? "[]");
          } catch {
            return [];
          }
        },
        (ids) => {
          try {
            localStorage.setItem(SEEN_KEY, JSON.stringify(ids));
          } catch {
            /* ignore */
          }
        },
      ),
      executors: {
        start: async (a) => {
          await startManagedAgent(a, { expectedSignerPubkey: pubkey });
        },
        stop: async (a) => {
          await stopManagedAgent(a);
        },
        message: async (a, channelId, text) => {
          await sendChannelMessage(
            channelId,
            text,
            null,
            undefined,
            [a],
            undefined,
            undefined,
            undefined,
            undefined,
            undefined,
            undefined,
            pubkey,
          );
        },
      },
      publishResult: (id, body) => p.publishResult(id, body),
      agentManagementAllowed: isAgentManagementAllowed,
      browser: makeBrowserDeps(
        () => playgroundBrowserHost,
        (id, shot) => p.publishShot(id, shot),
        () => syncBrowsers(true),
      ),
      onDrop: (reason, id) =>
        console.warn(`[bridge] ignored command ${id.slice(0, 8)}: ${reason}`),
    });
    let unsubCmd: (() => Promise<void>) | null = null;
    let cmdCancelled = false;
    // The command subscription used to be tried once: if the relay connection was still being (re)established when the app
    // loaded ("Relay connection attempt was superseded"), commands from the web were never received until the next reload.
    // Now it retries with backoff, and after every relay reconnect it re-fetches recent commands (the handler is idempotent).
    const cmdFilter = () => ({
      kinds: [BRIDGE_KIND],
      authors: [pubkey as string],
      "#t": [BRIDGE_TAG],
      since: nowS() - CMD_MAX_TTL_S - 60,
      limit: 50,
    });
    let cmdRetry: ReturnType<typeof setTimeout> | null = null;
    let cmdAttempt = 0;
    const subscribeCommands = () => {
      if (cmdCancelled) return;
      relayClient
        .subscribeLive(cmdFilter(), (e: RelayEvent) => void handler.handle(e))
        .then((u) => {
          cmdAttempt = 0;
          if (cmdCancelled) void u();
          else unsubCmd = u;
        })
        .catch((e) => {
          if (cmdCancelled) return;
          cmdAttempt++;
          const delay = Math.min(30_000, 1_000 * 2 ** Math.min(cmdAttempt, 5));
          console.warn(`[bridge] command subscription failed; retrying in ${delay}ms:`, e);
          cmdRetry = setTimeout(subscribeCommands, delay);
        });
    };
    subscribeCommands();
    const offCmdReconnect = relayClient.subscribeToReconnects(() => {
      relayClient
        .fetchEvents(cmdFilter())
        .then((evs: RelayEvent[]) => {
          for (const e of evs) void handler.handle(e);
        })
        .catch((e: unknown) => console.warn("[bridge] command catch-up failed:", e));
    });
    // Transcripts: recompute an agent's redacted entries at most once per second after observer activity.
    const dirty = new Set<string>();
    let flushTimer: ReturnType<typeof setTimeout> | null = null;
    const flushDirty = () => {
      flushTimer = null;
      for (const agent of dirty) {
        if (p.knowsAgent(agent))
          p.updateTranscript(agent, entriesFromItems(getAgentTranscript(agent)));
      }
      dirty.clear();
    };
    const unsubObserver = subscribeAgentObserverStore((update) => {
      if (!update?.agentPubkey) return;
      dirty.add(update.agentPubkey);
      flushTimer ??= setTimeout(flushDirty, 1000);
    });
    return () => {
      cmdCancelled = true;
      unsubRemote();
      unsubBrowsers();
      offCmdReconnect();
      if (cmdRetry) clearTimeout(cmdRetry);
      unsubGrants();
      clearInterval(syncInterval);
      if (syncTimer) clearTimeout(syncTimer);
      void unsubCmd?.();
      unsubObserver();
      if (flushTimer) clearTimeout(flushTimer);
      p.destroy();
      publisherRef.current = null;
    };
  }, [enabled, pubkey, relayUrl]);

  const agents = agentsQuery.data;
  React.useEffect(() => {
    const p = publisherRef.current;
    if (!p || !agents) return;
    p.update(
      buildAgentsSnapshot(
        agents.map((a) => ({
          pubkey: a.pubkey,
          name: a.name,
          status: a.status,
          pid: a.pid,
          runtime: a.runtime,
          model: a.model,
          personaId: a.personaId,
          lastStartedAt: a.lastStartedAt,
          lastError: a.lastError,
        })),
        channelTurns,
        (id) => nameRef.current(id),
        Date.now(),
      ),
    );
    // Publish what each running agent has already said (new agents/relaunch: backlog of the last entries).
    for (const a of agents)
      if (a.status === "running")
        p.updateTranscript(a.pubkey, entriesFromItems(getAgentTranscript(a.pubkey)));
  }, [agents, channelTurns]);
}
