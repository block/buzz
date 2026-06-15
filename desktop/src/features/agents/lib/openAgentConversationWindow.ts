import { WebviewWindow } from "@tauri-apps/api/webviewWindow";

import type { ChannelType, ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type OpenAgentConversationWindowInput = {
  agentPubkey: string;
  agentName: string;
  agentStatus: ManagedAgent["status"];
  channelId: string;
  channelName: string;
  channelType: ChannelType | null;
};

const DEFAULT_WIDTH = 460;
const DEFAULT_HEIGHT = 680;
const MIN_WIDTH = 380;
const MIN_HEIGHT = 460;

/**
 * Tauri window labels are restricted to `[a-zA-Z0-9-/:_]`. Derive a stable,
 * sanitized label from the channel + agent so re-opening the same conversation
 * focuses the existing window instead of spawning a duplicate.
 */
function windowLabel(channelId: string, agentPubkey: string): string {
  return `agent-${channelId}-${normalizePubkey(agentPubkey)}`.replace(
    /[^a-zA-Z0-9_-]/g,
    "-",
  );
}

/**
 * Open (or focus) a real OS window hosting the agent's live activity log and a
 * composer to chat with it. The window loads the same frontend at the
 * `/agent-window` route, which renders a chrome-less conversation view; the OS
 * provides the window chrome (title bar, traffic lights, resize).
 */
export async function openAgentConversationWindow(
  input: OpenAgentConversationWindowInput,
): Promise<void> {
  const label = windowLabel(input.channelId, input.agentPubkey);

  const existing = await WebviewWindow.getByLabel(label);
  if (existing) {
    await existing.unminimize().catch(() => {});
    await existing.show().catch(() => {});
    await existing.setFocus().catch(() => {});
    return;
  }

  const params = new URLSearchParams({
    channelId: input.channelId,
    pubkey: input.agentPubkey,
    name: input.agentName,
    channelName: input.channelName,
  });
  if (input.channelType) {
    params.set("channelType", input.channelType);
  }

  // Hash history: everything after `#` is the in-app location (path + search).
  const url = `index.html#/agent-window?${params.toString()}`;

  const win = new WebviewWindow(label, {
    url,
    title: `${input.agentName} · #${input.channelName}`,
    width: DEFAULT_WIDTH,
    height: DEFAULT_HEIGHT,
    minWidth: MIN_WIDTH,
    minHeight: MIN_HEIGHT,
    resizable: true,
    // Created hidden; App.tsx reveals it on mount to avoid an unstyled flash.
    visible: false,
  });

  win.once("tauri://error", (event) => {
    console.error("Failed to open agent conversation window", event.payload);
  });
}
