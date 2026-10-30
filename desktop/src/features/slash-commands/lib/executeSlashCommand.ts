import type { SlashInvocation } from "./recognizeSlashCommand";
import { SlashCommandCancelled } from "./slashCommandCancelled";
import { SlashUsageError } from "./slashUsageError";
import type { SlashCommandDefinition, SlashCommandRunContext } from "./types";

export type SlashCommandFailure = {
  title: string;
  description?: string;
};

export function slashCommandChannelOnlyError(name: string) {
  return `/${name} only works in channels.`;
}

export const SLASH_COMMAND_ATTACHMENT_ERROR =
  "Commands can't be sent with attachments. Remove the attachments to run it.";

/**
 * Runs one recognized command. `succeeded` means the caller clears the draft
 * and its attachments. `failed` and `cancelled` mean the caller keeps them;
 * a failure is always reported, a cancellation never is.
 */
export async function executeSlashCommand(params: {
  invocation: SlashInvocation<SlashCommandDefinition>;
  hasAttachments: boolean;
  isDm: boolean;
  createContext: (args: string) => Promise<SlashCommandRunContext>;
  reportFailure: (failure: SlashCommandFailure) => void;
}): Promise<"succeeded" | "failed" | "cancelled"> {
  const { invocation, hasAttachments, isDm, createContext, reportFailure } =
    params;
  const { command, args } = invocation;
  if (isDm && command.context === "channel") {
    reportFailure({ title: slashCommandChannelOnlyError(command.name) });
    return "failed";
  }
  if (hasAttachments) {
    reportFailure({ title: SLASH_COMMAND_ATTACHMENT_ERROR });
    return "failed";
  }
  try {
    const context = await createContext(args);
    await command.run(context);
    return "succeeded";
  } catch (error) {
    if (error instanceof SlashCommandCancelled) return "cancelled";
    if (error instanceof SlashUsageError) {
      reportFailure(
        error.detail
          ? { title: error.detail, description: `Usage: ${command.usage}` }
          : { title: `Usage: ${command.usage}` },
      );
    } else {
      reportFailure({
        title:
          error instanceof Error && error.message
            ? error.message
            : `Could not run /${command.name}.`,
      });
    }
    return "failed";
  }
}
