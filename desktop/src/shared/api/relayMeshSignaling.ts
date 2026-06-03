import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { relayClient } from "@/shared/api/relayClient";
import { canonicalPubkeyOrThrow } from "@/shared/lib/pubkey";
import {
  KIND_MESH_CALL_ME_NOW,
  KIND_MESH_CONNECT_REQUEST,
  KIND_MESH_STATUS_REPORT,
} from "@/shared/constants/kinds";

export async function publishMeshStatusReport(
  payload: Record<string, unknown>,
): Promise<void> {
  await relayClient.preconnect();
  const event = await signRelayEvent({
    kind: KIND_MESH_STATUS_REPORT,
    content: JSON.stringify(payload),
    tags: [],
  });
  await relayClient.publishEvent(
    event,
    "Timed out publishing mesh status.",
    "Failed to publish mesh status.",
  );
}

export async function publishMeshConnectRequest(input: {
  targetPubkey: string;
  selfEndpointAddr: string;
  peerEndpointAddr: string;
  attemptId: string;
  selfEndpointId?: string | null;
  peerEndpointId?: string | null;
}): Promise<void> {
  await relayClient.preconnect();
  const content: Record<string, unknown> = {
    v: 1,
    self_endpoint_addr: input.selfEndpointAddr,
    peer_endpoint_addr: input.peerEndpointAddr,
    attempt_id: input.attemptId,
  };
  if (input.selfEndpointId) content.self_endpoint_id = input.selfEndpointId;
  if (input.peerEndpointId) content.peer_endpoint_id = input.peerEndpointId;
  const event = await signRelayEvent({
    kind: KIND_MESH_CONNECT_REQUEST,
    content: JSON.stringify(content),
    tags: [["p", canonicalPubkeyOrThrow(input.targetPubkey)]],
  });
  await relayClient.publishEvent(
    event,
    "Timed out requesting mesh connection.",
    "Failed to request mesh connection.",
  );
}

export function subscribeToMeshCallMeNow(
  pubkey: string,
  onEvent: (event: RelayEvent) => void,
) {
  return relayClient.subscribeLive(
    {
      kinds: [KIND_MESH_CALL_ME_NOW],
      "#p": [pubkey],
      limit: 0,
      since: Math.max(0, Math.floor(Date.now() / 1_000) - 5),
    },
    onEvent,
  );
}
