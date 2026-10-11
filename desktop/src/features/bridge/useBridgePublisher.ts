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
import type { RelayEvent } from "@/shared/api/types";
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
        onError: (e) => console.warn("[bridge] publish failed:", e),
      },
    );
    publisherRef.current = p;
    p.start();
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
