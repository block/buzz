import { useSpawnerDirectory } from "../spawnerDirectoryStore";
import { usePendingSpawnerPromptUpdate } from "../spawnerPromptUpdateQueue";
import type { PersonaDropdownOption } from "./agentConfigOptions";
import { runtimeLabel, spawnerLabel } from "./ServerAgentsSection";
import {
  resolveServerAgentEditContext,
  serverModelOptions,
  type ServerAgentEditContext,
} from "./serverAgentEditPolicy";

/** Everything the Edit dialogs need to render a server-hosted agent. */
export type ServerAgentEditState = {
  /** Non-null when the agent being edited lives on a spawner. */
  context: ServerAgentEditContext | null;
  /** Friendly runtime name advertised by that spawner, when it advertised one. */
  runtime: string | undefined;
  /** A prompt edit is out but not yet confirmed by the spawner. */
  pending: boolean;
  /** Provider/model catalog, or null when the spawner advertised none. */
  ai: { providers: string[]; models: string[] } | null;
  providerOptions: PersonaDropdownOption[];
  modelOptions: PersonaDropdownOption[];
};

/**
 * Resolve server residency plus the spawner-advertised model catalog.
 *
 * Shared by the instance and definition dialogs: both need the same "is this
 * configured here or over there" answer, and both must offer the *host's*
 * models rather than whatever this Mac happens to have credentials for.
 */
export function useServerAgentEditContext(input: {
  relocatedToSpawner: string | null | undefined;
  deployedSpawnerPubkey: string | null;
  agentPubkey: string | null;
  slug: string | null;
  provider: string;
}): ServerAgentEditState {
  const directory = useSpawnerDirectory();
  const context = resolveServerAgentEditContext({
    relocatedToSpawner: input.relocatedToSpawner,
    deployedSpawnerPubkey: input.deployedSpawnerPubkey,
    agentPubkey: input.agentPubkey,
    slug: input.slug,
    spawnerNameFor: (pubkey) => spawnerLabel(pubkey, directory),
  });
  const promptUpdate = usePendingSpawnerPromptUpdate(
    context?.agentPubkey ?? "",
  );
  const announcement = context ? directory.get(context.spawnerPubkey) : null;
  const ai = context
    ? serverModelOptions(announcement?.ai, input.provider.trim() || null)
    : null;

  return {
    context,
    runtime: runtimeLabel(announcement?.runtime),
    pending: promptUpdate?.pending ?? false,
    ai,
    providerOptions: (ai?.providers ?? []).map((id) => ({
      label: id,
      value: id,
    })),
    modelOptions: (ai?.models ?? []).map((id) => ({ label: id, value: id })),
  };
}

/**
 * Append the current value as a "(current)" row when the spawner's catalog does
 * not list it.
 *
 * A spawner advertises what it can run today; an agent may already be
 * configured with something outside that list (an older catalog, a value set
 * from another device). Dropping it would render the field blank and make an
 * unrelated edit silently rewrite the value.
 */
export function withCurrentValueOption(
  options: PersonaDropdownOption[],
  value: string,
): PersonaDropdownOption[] {
  const trimmed = value.trim();
  if (!trimmed || options.some((option) => option.value === trimmed)) {
    return options;
  }
  return [...options, { label: `${trimmed} (current)`, value: trimmed }];
}
