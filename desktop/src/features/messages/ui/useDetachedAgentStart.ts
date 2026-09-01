import * as React from "react";
import { toast } from "sonner";
import { useStartManagedAgentMutation } from "@/features/agents/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { getErrorMessage } from "./useMentionSendFlow.helpers";

/**
 * Detached starts still in flight, keyed by `(scoped relay URL, pubkey)`.
 *
 * Awaiting the start used to make a duplicate unreachable: `isPending` was a
 * hard early return in the composer's send handler, so no second send could
 * begin, and by the time it lifted the mutation's `onSuccess` had written the
 * `running`/`deployed` record into the query cache. Detaching removes both —
 * for the whole in-flight window the cache still reads `stopped`, so a second
 * send re-fires. Module-level rather than a ref because the overlaps worth
 * collapsing include cross-composer ones (channel composer, thread panel,
 * `NewMessageScreen` each hold their own `useMentionSendFlow`).
 *
 * Keyed by relay as well as pubkey to mirror the backend's own runtime pair
 * key, so a start in one community can never suppress one in another — the
 * same tenant boundary `expectedRelayUrl` enforces below.
 */
const inFlightDetachedStarts = new Map<string, Promise<unknown>>();

/**
 * Drops every tracked in-flight start. Registered in `resetCommunityState`:
 * these starts belong to the community being left and are about to fail closed
 * at the backend's scope assertion anyway.
 */
export function resetDetachedAgentStarts(): void {
  inFlightDetachedStarts.clear();
}

/**
 * The backend fails a scope-mismatched start closed with a message ending in
 * "not sent". That reads wrong here: publish-first means the message *was*
 * published — only the wake was refused — so say what actually happened.
 */
function detachedStartFailureDetail(error: unknown): string {
  const message = getErrorMessage(error, "Could not start agent.");
  return message.includes("active community changed") ||
    message.includes("active identity changed")
    ? "You switched community or identity before it could start."
    : message;
}

/**
 * Fire-and-forget managed-agent start for the publish-first mention send,
 * bound to the tenant scope that was active when the send fired.
 *
 * Detaching the start means the call outlives the send, the channel, and —
 * since a community switch only remounts the React subtree — the community
 * itself. `start_managed_agent` resolves the workspace relay and the signing
 * identity at *execution* time, so an unscoped detached start can spawn or
 * deploy the agent against whichever tenant is active when it lands, carrying
 * the previous community's replay floor. The relay URL and the signing keys
 * change under separate locks during a switch, so both are captured (the
 * relay alone would still let the new identity act for the old tenant) and
 * `start_managed_agent` fails closed when either no longer matches. This is
 * the same binding `submitProjectAgentMessage` applies for the same
 * outlives-its-caller reason.
 *
 * Capture is per render: the callback closes over the community and identity
 * that were active when the composer last rendered, which is the send the
 * user pressed — never a value re-read after the switch it guards against.
 *
 * Returns whether this call actually fired a wake: a start already in flight
 * for the same agent in the same tenant is suppressed, since the wake is
 * per-agent rather than per-message and the first start's replay floor is
 * earlier than the second message. Callers use the result to report only real
 * fires (the send-perf summary counts them).
 */
export function useDetachedAgentStart(): (agent: ManagedAgent) => boolean {
  const startAgentMutateAsync = useStartManagedAgentMutation().mutateAsync;
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  // Handed over verbatim: `assert_expected_relay_scope` runs both sides
  // through `relay_http_base_url` (trim, strip trailing slash, ws→http), and
  // that comparison is case-sensitive. Lowercasing here — as the shared
  // storage-key normalizer does — would turn a stored `wss://Relay.Example`
  // into a permanent spurious mismatch that refuses every wake.
  const expectedRelayUrl = activeCommunity?.relayUrl || undefined;
  // The signer check is case-insensitive, so canonicalizing is free here.
  const expectedSignerPubkey = identityQuery.data?.pubkey
    ? normalizePubkey(identityQuery.data.pubkey)
    : undefined;
  return React.useCallback(
    (agent: ManagedAgent) => {
      // No synchronisation is needed or possible: the check, the call and the
      // registration below sit in one synchronous block, so no other send can
      // interleave between "is it in the set?" and "put it in the set".
      const key = `${expectedRelayUrl ?? ""}\u0000${normalizePubkey(agent.pubkey)}`;
      if (inFlightDetachedStarts.has(key)) {
        // One wake serves both messages. A local duplicate is a backend no-op
        // anyway, but a provider redeploy can replace a harness that had just
        // come up to answer the first message — and the user would get two
        // failure toasts for one problem.
        return false;
      }
      // Publish-first: the send no longer waits for the agent start. The
      // replay floor tells the spawned harness to replay at least back to
      // this moment, so the about-to-publish message is inside its first
      // subscription window however long the spawn takes.
      const replayFloorUnix = Math.floor(Date.now() / 1000);
      const started = startAgentMutateAsync({
        pubkey: agent.pubkey,
        expectedRelayUrl,
        expectedSignerPubkey,
        replayFloorUnix,
      })
        .catch((error: unknown) => {
          toast.error(
            `Could not start ${agent.name} — your message was sent, but the agent may not respond. ${detachedStartFailureDetail(
              error,
            )}`,
          );
        })
        .finally(() => {
          // Identity-guarded: `resetDetachedAgentStarts` may have cleared the
          // map and a later start re-registered this key while this one was in
          // flight (an A→B→A community switch), and an unguarded delete would
          // drop that newer entry. Clearing in `finally` rather than on success
          // is what keeps a failed start from latching the agent permanently.
          if (inFlightDetachedStarts.get(key) === started) {
            inFlightDetachedStarts.delete(key);
          }
        });
      inFlightDetachedStarts.set(key, started);
      return true;
    },
    [expectedRelayUrl, expectedSignerPubkey, startAgentMutateAsync],
  );
}
