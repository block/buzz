import { currentCompanionWindowKind } from "@/app/companionWindow";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Whether this webview is dedicated to an agent activity feed. */
export function isAgentActivityWindow(): boolean {
  return currentCompanionWindowKind() === "agent-activity";
}

/** Build the concise native title for a channel-scoped activity window. */
export function agentActivityWindowTitle(
  agentName: string,
  channelName: string,
): string {
  const normalizedAgentName = agentName.trim() || "Agent";
  const normalizedChannelName = channelName.trim().replace(/^#+/, "");
  return `${normalizedAgentName} · #${normalizedChannelName}`;
}

/** Keep a companion window's native title aligned with its resolved scope. */
export async function setAgentActivityWindowTitle(
  agentName: string,
  channelName: string,
): Promise<boolean> {
  if (!isAgentActivityWindow() || !channelName.trim()) return false;

  await getCurrentWindow().setTitle(
    agentActivityWindowTitle(agentName, channelName),
  );
  return true;
}
