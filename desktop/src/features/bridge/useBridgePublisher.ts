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
        ],
        onError: (e) => console.warn("[bridge] publish failed:", e),
      },
    );
    publisherRef.current = p;
    p.start();
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
      onDrop: (reason, id) =>
        console.warn(`[bridge] ignored command ${id.slice(0, 8)}: ${reason}`),
    });
    let unsubCmd: (() => Promise<void>) | null = null;
    let cmdCancelled = false;
    void relayClient
      .subscribeLive(
        {
          kinds: [BRIDGE_KIND],
          authors: [pubkey as string],
          "#t": [BRIDGE_TAG],
          since: nowS() - CMD_MAX_TTL_S - 60,
          limit: 50,
        },
        (e: RelayEvent) => void handler.handle(e),
      )
      .then((u) => {
        if (cmdCancelled) void u();
        else unsubCmd = u;
      })
      .catch((e) => console.warn("[bridge] command subscription failed:", e));
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
