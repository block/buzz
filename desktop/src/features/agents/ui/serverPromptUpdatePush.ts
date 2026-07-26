import { enqueueSpawnerPromptUpdate } from "../spawnerPromptUpdateQueue";
import type { ServerAgentEditContext } from "./serverAgentEditPolicy";

/**
 * Push a prompt/model/provider edit to the spawner after a successful
 * persona/instance save.
 *
 * Shared by `AgentDefinitionDialog` and `AgentInstanceEditDialog`: both save
 * a server-resident agent locally, then must also relay the same edit to the
 * spawner that actually runs it. Never throws — the queue owns delivery and
 * retries, so a failed send here must not surface as a failed save. Errors
 * are logged the same way `spawnerPromptUpdateQueue` logs its own send
 * failures (`console.debug`, module-prefixed).
 *
 * `context` accepts `null` so callers can pass their (possibly-null) resolved
 * edit context straight through without an `if` wrapper at the call site.
 */
export type ServerPromptUpdateSaved = {
  systemPrompt: string | null | undefined;
  model: string | null | undefined;
  provider: string | null | undefined;
};

export async function pushServerPromptUpdate(
  context: ServerAgentEditContext | null,
  saved: ServerPromptUpdateSaved,
): Promise<void> {
  if (!context) return;
  try {
    await enqueueSpawnerPromptUpdate({
      spawnerPubkey: context.spawnerPubkey,
      specSlug: context.specSlug,
      agentPubkey: context.agentPubkey,
      prompt: {
        system_prompt: saved.systemPrompt || undefined,
        model: saved.model || undefined,
        provider: saved.provider || undefined,
      },
    });
  } catch (error) {
    console.debug("[server-prompt-update-push] enqueue failed:", error);
  }
}

/**
 * Convenience wrapper for `AgentDefinitionDialog`'s edit-submit path: that
 * dialog's `onSubmit` prop resolves to `unknown` (its callers agree on a
 * `boolean` success/failure convention without the type enforcing it — see
 * task-8-report.md), so the "did the save actually succeed" check is folded
 * in here to keep the call site a single line.
 */
export async function pushServerPromptUpdateAfterSubmit(
  context: ServerAgentEditContext | null,
  submitResult: unknown,
  saved: ServerPromptUpdateSaved,
): Promise<void> {
  if (submitResult === false) return;
  await pushServerPromptUpdate(context, saved);
}

/**
 * Short-named variant for `AgentInstanceEditDialog`'s save path, which
 * already has model/provider bundled in its own `inheritedSubmission`
 * snapshot (see `resolveInheritedRuntimeSubmission`) — this just adds
 * `systemPrompt` alongside it without requiring an intermediate object.
 */
export async function pushPrompt(
  context: ServerAgentEditContext | null,
  systemPrompt: string | null | undefined,
  modelProvider: {
    model: string | null | undefined;
    provider: string | null | undefined;
  },
): Promise<void> {
  await pushServerPromptUpdate(context, { systemPrompt, ...modelProvider });
}
