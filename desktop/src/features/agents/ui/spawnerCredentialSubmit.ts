import type { SpawnerCredentialAck } from "@/shared/api/tauriSpawner";

/** Outcome of one credential save/clear round-trip, for the card to render. */
export type CredentialSubmitResult =
  | { kind: "saved"; cleared: boolean }
  | { kind: "error"; message: string };

/** What `submitSpawnerCredential` needs from the transport layer. */
export type CredentialSubmitDeps = {
  /** Publish the encrypted update (see `sendSpawnerCredentialUpdate`). */
  send: (input: { spawnerPubkey: string; credential: string }) => Promise<void>;
  /** Await the spawner's encrypted ack (see `waitForSpawnerCredentialAck`). */
  waitForAck: (
    spawnerPubkey: string,
    timeoutMs: number,
  ) => Promise<SpawnerCredentialAck>;
};

/**
 * Send a credential (or an empty string to clear) and wait for the spawner's
 * ack.
 *
 * The waiter is registered before the send so an ack that races the publish
 * round-trip cannot be missed. Errors are folded into the result rather than
 * thrown, so the card's render logic is a plain switch on `kind`.
 */
export async function submitSpawnerCredential(
  deps: CredentialSubmitDeps,
  spawnerPubkey: string,
  credential: string,
  timeoutMs: number,
): Promise<CredentialSubmitResult> {
  try {
    const acked = deps.waitForAck(spawnerPubkey, timeoutMs);
    // A rejection from the waiter (timeout, community switch) must not become
    // an unhandled rejection while the send itself is failing.
    acked.catch(() => {});
    await deps.send({ spawnerPubkey, credential });
    const ack = await acked;
    if (!ack.accepted) {
      return {
        kind: "error",
        message: ack.message || "The server rejected the credential.",
      };
    }
    return { kind: "saved", cleared: credential === "" };
  } catch (error) {
    return {
      kind: "error",
      message:
        error instanceof Error
          ? error.message
          : "Failed to send the credential.",
    };
  }
}
