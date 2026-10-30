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
import { KIND_SET_THREAD_POLICY } from "@/shared/constants/kinds";

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

/**
 * NIP-TP community thread policy. `nested` keeps each reply under the parent
 * it was sent to; `flat` presents every reply as a direct reply to the thread
 * root.
 */
export type ThreadPolicy = "flat" | "nested";

/**
 * Fetch a community's thread policy from its relay's NIP-11 document. The
 * backend maps a missing or unknown value to `nested`; an unreachable relay
 * rejects so the query can retry.
 */
export async function fetchCommunityThreadPolicy(
  relayUrl: string,
): Promise<ThreadPolicy> {
  const policy = await invokeTauri<string>("fetch_thread_policy", {
    relayUrl,
  });
  return policy === "flat" ? "flat" : "nested";
}

/**
 * Publish a kind:9034 command setting the active community's thread policy.
 * Requires relay admin/owner role — the relay rejects the command otherwise.
 */
export async function setCommunityThreadPolicy(
  policy: ThreadPolicy,
): Promise<void> {
  const event = await signRelayEvent({
    kind: KIND_SET_THREAD_POLICY,
    content: "",
    tags: [["thread_policy", policy]],
  });
  await relayClient.publishEvent(
    event,
    "Timed out while updating thread replies.",
    "Failed to update thread replies.",
  );
}
