import { relayClient } from "@/shared/api/relayClient";
import { ReadOnlyRelayClient } from "@/shared/api/readOnlyRelayClient";
import { relayRequiresMembership } from "@/shared/api/relayMembers";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { isRelayUnreachableError } from "@/shared/lib/relayError";

export const KIND_NIP43_LEAVE_REQUEST = 28936;

/** Offer a device-only fallback for unavailable relays, not permission failures. */
export function canRemoveCommunityLocally(error: unknown): boolean {
  if (isRelayUnreachableError(error)) return true;
  const message = error instanceof Error ? error.message : error;
  return (
    typeof message === "string" &&
    (/^relay returned (?:404|410|5\d\d)\b/.test(message) ||
      message === "Relay connection closed." ||
      message === "Relay connection errored." ||
      message === "Failed to connect to relay." ||
      message === "Relay authentication timed out." ||
      message.startsWith("Timed out while leaving the community.") ||
      message.startsWith("Couldn't send the leave request."))
  );
}

type LeaveCommunityDependencies = {
  requiresMembership: (relayUrl: string) => Promise<boolean>;
  sign: typeof signRelayEvent;
  publishActive: (event: RelayEvent) => Promise<unknown>;
  createRelayClient: (relayUrl: string) => {
    publishEvent: (event: RelayEvent) => Promise<unknown>;
    disconnect: () => void;
  };
};

const defaultDependencies: LeaveCommunityDependencies = {
  requiresMembership: relayRequiresMembership,
  sign: signRelayEvent,
  publishActive: (event) =>
    relayClient.publishEvent(
      event,
      "Timed out while leaving the community. Try again.",
      "Couldn't send the leave request. Check your connection and try again.",
    ),
  createRelayClient: (relayUrl) => new ReadOnlyRelayClient(relayUrl),
};

function membershipIsAlreadyAbsent(error: unknown): boolean {
  return (
    error instanceof Error &&
    error.message.toLowerCase().includes("not a relay member")
  );
}

export type LeaveCommunityResult =
  | { status: "left" }
  | { status: "already-absent" };

async function publishLeaveRequest(
  publish: () => Promise<unknown>,
): Promise<LeaveCommunityResult> {
  try {
    await publish();
    return { status: "left" };
  } catch (error) {
    if (!membershipIsAlreadyAbsent(error)) throw error;
    return { status: "already-absent" };
  }
}

/** Revoke relay membership and resolve only after the relay accepts the request. */
export async function leaveCommunity(
  relayUrl: string,
  activeRelayUrl: string | undefined,
  dependencies: LeaveCommunityDependencies = defaultDependencies,
): Promise<LeaveCommunityResult> {
  if (!(await dependencies.requiresMembership(relayUrl))) {
    return { status: "left" };
  }

  const event = await dependencies.sign({
    kind: KIND_NIP43_LEAVE_REQUEST,
    content: "",
    tags: [["-"]],
  });

  if (relayUrl === activeRelayUrl) {
    return publishLeaveRequest(() => dependencies.publishActive(event));
  }

  const client = dependencies.createRelayClient(relayUrl);
  try {
    return await publishLeaveRequest(() => client.publishEvent(event));
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
