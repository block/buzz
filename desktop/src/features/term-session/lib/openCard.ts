import { isTauri } from "@tauri-apps/api/core";
import { getTerminalBackend } from "@/features/terminal/terminalClient";
import { withEnvExports } from "@/features/bridge/termHandoff";
import { toast } from "sonner";
import { ensureProxyDirectLive } from "@/features/bridge/remoteCmd";
import { openTerminalPanel } from "@/features/terminal/terminalPanelStore";

import type { TermSessionCard } from "./types.ts";
import {
  buildLaunchFromPrepare,
  prepareTermSessionLaunch,
} from "./prepareLaunch.ts";
import { requestTermSessionOpen } from "./openRequestStore.ts";

export type OpenTermSessionCardContext = {
  channelId: string;
  channelName: string;
  threadId: string | null;
};

/**
 * Open Buzz Term for this card: prepare prompt (+ OpenClaw MCP when flagged),
 * show the panel, and ask TerminalBootstrap to attach/focus or launch.
 *
 * Re-Open: if a live Term tab for `sid` still exists, Bootstrap focuses it.
 * Otherwise we prepare again and relaunch (grant reuse via load_grant only).
 */
export async function openTermSessionCard(
  card: TermSessionCard,
  context: OpenTermSessionCardContext,
): Promise<{ launchCommand: string; extraEnv: Record<string, string> } | null> {
  const web = !isTauri();
  if (web && !getTerminalBackend()) {
    toast.error(
      "Opening this session needs Buzz Term on your desktop: turn on Allow remote terminal there and connect this device over Tailscale.",
    );
    return null;
  }
  let prepared;
  // Web: same shared Direct client the left nav binds (and the Agents/Settings pages connect); connect it if it isn't yet.
  if (web) await ensureProxyDirectLive(8000);
  try {
    // Web: answered by the desktop over the Direct connection (directCmd), the same command its own Open runs.
    prepared = await prepareTermSessionLaunch(card);
  } catch (error) {
    const raw =
      error instanceof Error ? error.message : String(error ?? "Launch failed");
    const code = (error as { code?: string } | null)?.code;
    toast.error(
      web && (code === "needs_direct" || /Direct connection|not available in the web app/i.test(raw))
        ? "Opening a Buzz Term session from the web needs a Direct connection to your desktop. Open the Agents page or Settings to connect, then try again."
        : raw,
    );
    return null;
  }

  if (card.tool === "claude" && prepared.buzzDevMcpWired === false) {
    toast.message(
      "buzz-dev-mcp user-signer was not wired; channel/thread MCP tools may be unavailable.",
    );
  }

  if (
    card.openclawWorkspace &&
    card.tool === "claude" &&
    !prepared.openclawWired
  ) {
    toast.error(
      "OpenClaw workspace is enabled on this card, but the MCP grant could not be wired.",
    );
    return null;
  }

  if (card.openclawWorkspace && card.tool === "codex") {
    toast.message(
      "OpenClaw workspace MCP is Claude-first; Codex launch continues without MCP wiring.",
    );
  }

  const built = buildLaunchFromPrepare(card, prepared);
  // A remote (web) terminal is a fresh shell on the desktop with no spawn-time env, so the env goes into the typed line.
  const launchCommand = web
    ? withEnvExports(built.launchCommand, built.extraEnv)
    : built.launchCommand;
  const extraEnv = built.extraEnv;
  requestTermSessionOpen({
    sid: card.sid,
    channelId: context.channelId,
    channelName: context.channelName,
    threadId: context.threadId,
    tool: card.tool,
    cwd: card.cwd,
    promptPath: prepared.promptPath,
    launchCommand,
    extraEnv,
  });
  if (web) openTerminalPanel("maximized", "all");
  return { launchCommand, extraEnv };
}
