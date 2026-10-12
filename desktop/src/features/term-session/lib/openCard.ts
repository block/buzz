import { toast } from "sonner";

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
  let prepared;
  try {
    prepared = await prepareTermSessionLaunch(card);
  } catch (error) {
    const message =
      error instanceof Error ? error.message : String(error ?? "Launch failed");
    toast.error(message);
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

  const { launchCommand, extraEnv } = buildLaunchFromPrepare(card, prepared);
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
  return { launchCommand, extraEnv };
}
