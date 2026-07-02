/**
 * Workspace icon, Buzz extension to NIP-43 + standard NIP-11 `icon`.
 *
 * An admin/owner publishes a kind:9033 command carrying the icon in an
 * `["icon", value]` tag; the relay validates the sender's relay role and
 * stores the icon per community, serving it in the standard `icon` field of
 * its NIP-11 relay information document. Every member's client reads NIP-11,
 * so the whole workspace sees the same icon.
 *
 * The icon value is a small `data:image/*` URL (downscaled client-side
 * before publish) so it renders for INACTIVE workspaces straight from the
 * document — no cross-relay media fetch behind another relay's auth wall.
 */

import { relayClient } from "@/shared/api/relayClient";
import { invokeTauri, signRelayEvent } from "@/shared/api/tauri";

/** Buzz: admin command to set the workspace profile (icon). */
export const KIND_SET_WORKSPACE_PROFILE = 9033;

/**
 * Fetch a workspace's icon from its relay's NIP-11 document (plain
 * unauthenticated HTTP via the Tauri backend — works for inactive
 * workspaces too). Unreachable relay or no icon → null.
 */
export async function fetchWorkspaceIcon(
  relayUrl: string,
): Promise<string | null> {
  const icon = await invokeTauri<string | null>("fetch_workspace_icon", {
    relayUrl,
  });
  return icon || null;
}

/**
 * Publish a kind:9033 command setting (or clearing, with "") the workspace
 * icon on the active relay. Requires relay admin/owner role — the relay
 * rejects the command otherwise.
 */
export async function setWorkspaceIcon(icon: string): Promise<void> {
  const event = await signRelayEvent({
    kind: KIND_SET_WORKSPACE_PROFILE,
    content: "",
    tags: [["icon", icon]],
  });
  await relayClient.publishEvent(
    event,
    "Timed out while updating the workspace icon.",
    "Failed to update the workspace icon.",
  );
}
