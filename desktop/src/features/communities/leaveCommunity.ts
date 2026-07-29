import { relayClient } from "@/shared/api/relayClient";
import { ReadOnlyRelayClient } from "@/shared/api/readOnlyRelayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";

export const KIND_NIP43_LEAVE_REQUEST = 28936;

type LeaveCommunityDependencies = {
  sign: typeof signRelayEvent;
  publishActive: (event: RelayEvent) => Promise<unknown>;
  createRelayClient: (relayUrl: string) => {
    publishEvent: (event: RelayEvent) => Promise<unknown>;
    disconnect: () => void;
  };
};

const defaultDependencies: LeaveCommunityDependencies = {
  sign: signRelayEvent,
  publishActive: (event) =>
    relayClient.publishEvent(
      event,
      "Timed out while leaving the community. Try again.",
      "Failed to send the leave request. Check your connection and try again.",
    ),
  createRelayClient: (relayUrl) => new ReadOnlyRelayClient(relayUrl),
};

/** Revoke relay membership and resolve only after the relay accepts the request. */
export async function leaveCommunity(
  relayUrl: string,
  activeRelayUrl: string | undefined,
  dependencies: LeaveCommunityDependencies = defaultDependencies,
): Promise<void> {
  const event = await dependencies.sign({
    kind: KIND_NIP43_LEAVE_REQUEST,
    content: "",
    tags: [["-"]],
  });

  if (relayUrl === activeRelayUrl) {
    await dependencies.publishActive(event);
    return;
  }

  const client = dependencies.createRelayClient(relayUrl);
  try {
    await client.publishEvent(event);
  } catch (error) {
    if (
      error instanceof Error &&
      error.message.toLowerCase().includes("timed out")
    ) {
      throw new Error("Timed out while leaving the community. Try again.");
    }
    throw error;
  } finally {
    client.disconnect();
  }
}
