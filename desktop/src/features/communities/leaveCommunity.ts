import { relayClient } from "@/shared/api/relayClient";
import { ReadOnlyRelayClient } from "@/shared/api/readOnlyRelayClient";
import { relayRequiresMembership } from "@/shared/api/relayMembers";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";

export const KIND_NIP43_LEAVE_REQUEST = 28936;

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

/**
 * The shared session latches terminal when the relay rejects AUTH and then
 * refuses to reconnect, so it reports its own latch rather than the relay's
 * answer. It cannot deliver the leave request in that state.
 */
function sessionIsTerminal(error: unknown): boolean {
  return (
    error instanceof Error &&
    error.message.toLowerCase().includes("session is terminal")
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
    try {
      return await publishLeaveRequest(() => dependencies.publishActive(event));
    } catch (error) {
      // A terminal session is most often a membership the relay already ended,
      // and then waiting for it to accept a leave request is a dead end (#7049).
      // Fall through to a fresh connection so the relay itself answers: an ended
      // membership resolves as already-absent, anything else still fails.
      if (!sessionIsTerminal(error)) throw error;
    }
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
