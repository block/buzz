/**
 * Community profile, Buzz extension to NIP-43 + standard NIP-11.
 *
 * An admin/owner publishes a kind:9033 command carrying profile fields in
 * tags; the relay validates the sender's relay role and stores them per
 * community, serving them from its NIP-11 relay information document. Every
 * member's client reads NIP-11, so the whole community sees the same settings.
 *
 * The icon value is a small `data:image/*` URL (downscaled client-side
 * before publish) so it renders for INACTIVE communities straight from the
 * document — no cross-relay media fetch behind another relay's auth wall.
 */

import { relayClient } from "@/shared/api/relayClient";
import { invokeTauri, signRelayEvent } from "@/shared/api/tauri";

/** Buzz: admin command to set the community profile (icon). */
export const KIND_SET_COMMUNITY_PROFILE = 9033;

export type CommunityProfile = {
  icon: string | null;
  threadRepliesInChannel: boolean;
};

export const communityProfileQueryKey = (relayUrl: string) =>
  ["community-profile", relayUrl] as const;

/**
 * Fetch a community's profile from its relay's NIP-11 document (plain
 * unauthenticated HTTP via the Tauri backend — works for inactive
 * communities too). Unreachable relay or malformed data → defaults.
 */
export async function fetchCommunityProfile(
  relayUrl: string,
): Promise<CommunityProfile> {
  const profile = await invokeTauri<{
    icon?: string | null;
    threadRepliesInChannel?: boolean;
  }>("fetch_workspace_profile", {
    relayUrl,
  });
  return {
    icon: profile.icon || null,
    threadRepliesInChannel: profile.threadRepliesInChannel === true,
  };
}

/**
 * Fetch a community's icon from its relay's NIP-11 document (plain
 * unauthenticated HTTP via the Tauri backend — works for inactive
 * communities too). Unreachable relay or no icon → null.
 */
export async function fetchCommunityIcon(
  relayUrl: string,
): Promise<string | null> {
  return (await fetchCommunityProfile(relayUrl)).icon;
}

/**
 * Publish a kind:9033 command setting (or clearing, with "") the community
 * icon on the active relay. Requires relay admin/owner role — the relay
 * rejects the command otherwise.
 */
export async function setCommunityIcon(icon: string): Promise<void> {
  const event = await signRelayEvent({
    kind: KIND_SET_COMMUNITY_PROFILE,
    content: "",
    tags: [["icon", icon]],
  });
  await relayClient.publishEvent(
    event,
    "Timed out while updating the community icon.",
    "Failed to update the community icon.",
  );
}

/**
 * Publish a kind:9033 command setting whether thread replies are projected
 * into the main channel feed. Requires relay admin/owner role.
 */
export async function setCommunityThreadRepliesInChannel(
  enabled: boolean,
): Promise<void> {
  const event = await signRelayEvent({
    kind: KIND_SET_COMMUNITY_PROFILE,
    content: "",
    tags: [["thread_replies_in_channel", enabled ? "true" : "false"]],
  });
  await relayClient.publishEvent(
    event,
    "Timed out while updating thread reply display.",
    "Failed to update thread reply display.",
  );
}
