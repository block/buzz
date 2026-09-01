import * as React from "react";
import { toast } from "sonner";
import { useStartManagedAgentMutation } from "@/features/agents/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { getErrorMessage } from "./useMentionSendFlow.helpers";

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
 */
export function useDetachedAgentStart(): (agent: ManagedAgent) => void {
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
      // Publish-first: the send no longer waits for the agent start. The
      // replay floor tells the spawned harness to replay at least back to
      // this moment, so the about-to-publish message is inside its first
      // subscription window however long the spawn takes.
      const replayFloorUnix = Math.floor(Date.now() / 1000);
      void startAgentMutateAsync({
        pubkey: agent.pubkey,
        expectedRelayUrl,
        expectedSignerPubkey,
        replayFloorUnix,
      }).catch((error: unknown) => {
        toast.error(
          `Could not start ${agent.name} — your message was sent, but the agent may not respond. ${detachedStartFailureDetail(
            error,
          )}`,
        );
      });
    },
    [expectedRelayUrl, expectedSignerPubkey, startAgentMutateAsync],
  );
}
