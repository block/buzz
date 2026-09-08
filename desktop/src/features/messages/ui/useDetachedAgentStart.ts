import * as React from "react";
import { toast } from "sonner";
import { useStartManagedAgentMutation } from "@/features/agents/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { matchesDetachedToastScope } from "@/features/messages/lib/detachedToastScope";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { getErrorMessage } from "./useMentionSendFlow.helpers";

/**
 * Detached starts still in flight, keyed by the full tenant scope the wake
 * asserts: `(scoped relay URL, expected signer, agent pubkey)`.
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
 * The key is the asserted scope, not the backend's `ManagedAgentRuntimeKey`
 * (which is `(pubkey, relay_url)` — it tracks *runtimes*, while this map tracks
 * scoped start *operations*). `start_managed_agent` asserts relay **and**
 * signer before it spawns or deploys, so coalescing on the relay alone would
 * let a start held under one signing identity suppress another identity's wake
 * for the same agent on the same relay — a mid-session key import (the
 * membership-denied overlay writes a new identity straight into the live query
 * cache) inside a deploy window is enough to reach it. Both halves are in the
 * key, so a wake is only ever suppressed by one asserting exactly the same
 * scope.
 *
 * Entries deliberately survive community switches (`resetCommunityState` does
 * NOT clear this map). A held start is not invalidated by leaving its
 * community: the backend's scope assertion is a current-state check, so a
 * start fired in A is valid again the moment A is re-applied — an A→B→A
 * round-trip that cleared the map would let a second send duplicate a
 * provider deploy the first start is still performing (and hand the harness
 * the second message's replay floor, past the first message). The key is the
 * tenant scope, so a retained entry cannot affect another community, and the
 * `finally` below self-cleans on settlement, which the deploy op bounds.
 */
const inFlightDetachedStarts = new Map<string, Promise<unknown>>();

/**
 * Drops every tracked in-flight start. Test-only isolation seam — one test's
 * held start must not suppress the next test's. Production deliberately never
 * calls this: see the map's doc for why entries survive community switches.
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
 * The one wording for "the wake did not happen". The send path flushes its
 * queued wakes only after the relay accepts the publish, so whenever this
 * toast can appear the message really was sent — both the refused-before-
 * firing and the failed-after-firing cases owe the user the same warning,
 * differing only in the detail that follows it.
 */
function warnAgentMayNotRespond(agentName: string, detail: string): void {
  toast.error(
    `Could not start ${agentName} — your message was sent, but the agent may not respond. ${detail}`,
  );
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
 * If either half of that scope is not yet known — no active community, or an
 * identity query that has not resolved — the wake is refused rather than fired
 * unscoped. The backend reads a missing value as "no assertion", so an
 * unscoped detached start is exactly the cross-tenant spawn this hook exists
 * to prevent, and its dedupe key would collapse to a relay-less one shared
 * across communities. Waiting for the query instead of refusing is not an
 * option: reading the scope once it resolves is a post-send read, which is the
 * thing the per-render capture rules out. Refusing is visible (a toast, and a
 * `false` return) and the user's next send re-fires it.
 *
 * Returns whether this call actually fired a wake: a start already in flight
 * for the same agent under the same asserted scope — relay *and* signer — is
 * suppressed, since the wake is per-agent rather than per-message and the
 * first start's replay floor is earlier than the second message.
 *
 * `replayFloorUnix` lets a caller pass a floor captured earlier than this
 * call — the send path queues its wakes during preparation and flushes them
 * only after the publish succeeds, so the floor must be the enqueue-time
 * capture (≤ the message's `created_at` by construction), not flush time,
 * which could exceed it and push the harness's startup watermark past the
 * very message the floor exists to cover. Callers with no queued floor omit
 * it and get fire time.
 */
export function useDetachedAgentStart(): (
  agent: ManagedAgent,
  replayFloorUnix?: number,
) => boolean {
  const startAgentMutateAsync = useStartManagedAgentMutation().mutateAsync;
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  // Handed over verbatim: `assert_expected_relay_scope` runs both sides
  // through `relay_http_base_url` (trim, strip trailing slash, ws→http), and
  // that comparison is case-sensitive. Lowercasing here — as the shared
  // storage-key normalizer does — would turn a stored `wss://Relay.Example`
  // into a permanent spurious mismatch that refuses every wake. Emptiness is
  // judged on the trimmed form because the backend does the same, and reads a
  // blank scope as no assertion at all.
  const expectedRelayUrl = activeCommunity?.relayUrl?.trim()
    ? activeCommunity.relayUrl
    : undefined;
  // The signer check is case-insensitive, so canonicalizing is free here.
  const expectedSignerPubkey =
    normalizePubkey(identityQuery.data?.pubkey ?? "") || undefined;
  return React.useCallback(
    (agent: ManagedAgent, replayFloorUnix?: number) => {
      if (!expectedRelayUrl || !expectedSignerPubkey) {
        // Fail closed: an unscoped start resolves the relay and the signing
        // identity at execution time, so it can land on whichever tenant is
        // active by then — and its dedupe key would be shared across
        // communities.
        warnAgentMayNotRespond(
          agent.name,
          "Buzz is still connecting to this community — mention the agent again in a moment.",
        );
        return false;
      }
      // No synchronisation is needed or possible: the check, the call and the
      // registration below sit in one synchronous block, so no other send can
      // interleave between "is it in the set?" and "put it in the set".
      //
      // Relay verbatim, because its backend comparison is case-sensitive;
      // signer and agent normalized. `expectedSignerPubkey` is already
      // canonicalized at capture, matching `assert_expected_signer`'s
      // case-insensitive compare, so two casings of one identity cannot split
      // the key.
      const key = `${expectedRelayUrl}\u0000${expectedSignerPubkey}\u0000${normalizePubkey(agent.pubkey)}`;
      if (inFlightDetachedStarts.has(key)) {
        // One wake serves both messages. A local duplicate is a backend no-op
        // anyway, but a provider redeploy can replace a harness that had just
        // come up to answer the first message — and the user would get two
        // failure toasts for one problem.
        return false;
      }
      // Publish-first: the send no longer waits for the agent start. The
      // replay floor tells the spawned harness to replay at least back to
      // the message that wanted this wake — the enqueue-time capture when the
      // send path queued it, or this moment for a caller with no queue — so
      // that message is inside the harness's first subscription window
      // however long the spawn takes.
      const started = startAgentMutateAsync({
        pubkey: agent.pubkey,
        expectedRelayUrl,
        expectedSignerPubkey,
        replayFloorUnix: replayFloorUnix ?? Math.floor(Date.now() / 1000),
      })
        .catch((error: unknown) => {
          // This settles arbitrarily long after the send, and `<Toaster />`
          // mounts outside the community remount boundary — so an unfenced
          // warning would render this community's agent name and error detail
          // over whichever community is on screen by then. Deliver only while
          // the scope captured above is the one being looked at; on an A→B→A
          // round-trip the mirror matches again and the warning lands exactly
          // where re-mentioning the agent is possible. Suppression is logged,
          // not reworded: any wording still names another community's agent.
          if (
            !matchesDetachedToastScope(expectedRelayUrl, expectedSignerPubkey)
          ) {
            console.warn(
              `[useDetachedAgentStart] suppressed a start-failure warning for ${agent.name}: the community it was fired in is no longer on screen`,
              error,
            );
            return;
          }
          warnAgentMayNotRespond(agent.name, detachedStartFailureDetail(error));
        })
        .finally(() => {
          // Identity-guarded: only test isolation clears this map now
          // (production retains entries across community switches), but if a
          // reset-and-re-register did interleave while this start was in
          // flight, an unguarded delete would drop the newer entry. Clearing
          // in `finally` rather than on success is what keeps a failed start
          // from latching the agent permanently.
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
