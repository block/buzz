// Mounted once in the desktop AppShell (no-op in the browser). Feeds the bridge publisher from existing agent state.
import { isTauri } from "@tauri-apps/api/core";
import * as React from "react";

import { useActiveAgentTurnsByChannel } from "@/features/agents/activeAgentTurnsStore";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { nip44EncryptToSelf, signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { BridgePublisher } from "./publisher";
import { buildAgentsSnapshot } from "./snapshot";

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
    return () => {
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
  }, [agents, channelTurns]);
}
