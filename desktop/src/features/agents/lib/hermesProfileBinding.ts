type HermesProfileAgentShape = {
  runtime?: string | null;
  agentCommand?: string | null;
  agentArgs?: readonly string[] | null;
};

const PROFILE_NAME = /^[a-z0-9][a-z0-9_-]{0,63}$/;

/** Return the local Hermes profile bound through the managed instance args. */
export function hermesProfileNameFromAgent(
  agent: HermesProfileAgentShape,
): string | null {
  const runtime = agent.runtime?.trim();
  const command = (agent.agentCommand ?? "")
    .trim()
    .split(/[\\/]/)
    .at(-1)
    ?.toLowerCase();
  if (runtime !== "hermes" && command !== "hermes-acp") return null;

  const args = agent.agentArgs ?? [];
  const flagIndex = args.indexOf("--profile");
  if (flagIndex < 0) return null;
  const profileName = args[flagIndex + 1]?.trim() ?? "";
  return PROFILE_NAME.test(profileName) ? profileName : null;
}
