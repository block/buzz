export type DirectAgentCreationResult = {
  requestId: string;
  status: "created" | "denied" | "failed";
  displayName: string;
  agentPubkey?: string;
  message: string;
};

const RESULT_PREFIX = "<!-- buzz-agent-create-result ";
const RESULT_SUFFIX = " -->";

function visibleText(value: string, maxCharacters: number): string {
  return [...value]
    .filter((character) => {
      const code = character.codePointAt(0) ?? 0;
      return code >= 32 && code !== 127;
    })
    .join("")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .slice(0, maxCharacters);
}

export function directAgentCreationResultContent(
  result: DirectAgentCreationResult,
): string {
  const visible =
    result.status === "created"
      ? `Created **${visibleText(result.displayName, 120)}** and added it to this channel.`
      : `Could not directly create **${visibleText(result.displayName, 120)}**: ${visibleText(result.message, 500)}`;
  const marker = [
    `request=${result.requestId}`,
    `status=${result.status}`,
    ...(result.agentPubkey ? [`pubkey=${result.agentPubkey}`] : []),
  ].join(" ");
  return `${visible}\n\n${RESULT_PREFIX}${marker}${RESULT_SUFFIX}`;
}
