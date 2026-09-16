/**
 * Community icon, Buzz extension to NIP-43 + standard NIP-11 `icon`.
 *
 * An admin/owner publishes a kind:9033 command carrying the icon in an
 * `["icon", value]` tag; the relay validates the sender's relay role and
 * stores the icon per community, serving it in the standard `icon` field of
 * its NIP-11 relay information document. Every member's client reads NIP-11,
 * so the whole community sees the same icon.
 *
 * The icon value is a small `data:image/*` URL (downscaled client-side
 * before publish) so it renders for INACTIVE communities straight from the
 * document — no cross-relay media fetch behind another relay's auth wall.
 */

import { relayClient } from "@/shared/api/relayClient";
import { invokeTauri, signRelayEvent } from "@/shared/api/tauri";

/** Buzz: admin command to set the community profile (icon). */
export const KIND_SET_COMMUNITY_PROFILE = 9033;

/**
 * Fetch a community's icon from its relay's NIP-11 document (plain
 * unauthenticated HTTP via the Tauri backend — works for inactive
 * communities too). Unreachable relay or no icon → null.
 */
export async function fetchCommunityIcon(
  relayUrl: string,
): Promise<string | null> {
  const icon = await invokeTauri<string | null>("fetch_workspace_icon", {
    relayUrl,
  });
  return icon || null;
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

export type CommunityProfile = { name: string | null };

/** An absent descriptor is an older relay, not a successful empty name. */
export async function fetchCommunityProfile(
  relayUrl: string,
): Promise<CommunityProfile | null> {
  const value = await invokeTauri<unknown>("fetch_workspace_profile", {
    relayUrl,
  });
  if (value === null) return null;
  if (!value || typeof value !== "object" || !("name" in value)) {
    throw new Error("Invalid community profile response.");
  }
  const name = value.name;
  if (
    name !== null &&
    (typeof name !== "string" ||
      !name.trim() ||
      new TextEncoder().encode(name).length > 256)
  ) {
    throw new Error("Invalid community name response.");
  }
  return { name: typeof name === "string" ? name.trim() : null };
}

/** Publish only the name: existing icons are preserved by supporting relays. */
export async function setCommunityName(
  name: string,
  relayUrl: string,
  isCurrent: () => boolean,
): Promise<void> {
  // Never send a name-only command to an old relay: it interprets a missing
  // icon tag as a request to clear the icon.
  if (!(await fetchCommunityProfile(relayUrl))) {
    throw new Error(
      "This relay needs an update before it can share community names.",
    );
  }
  if (!isCurrent())
    throw new Error("Community changed. Reopen its settings to rename it.");
  const event = await signRelayEvent({
    kind: KIND_SET_COMMUNITY_PROFILE,
    content: "",
    tags: [["name", name.trim()]],
  });
  if (!isCurrent())
    throw new Error("Community changed. Reopen its settings to rename it.");
  await relayClient.publishEvent(
    event,
    "Timed out while updating the community name.",
    "Failed to update the community name.",
  );
}
