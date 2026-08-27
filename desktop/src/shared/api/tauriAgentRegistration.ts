import { invokeTauri } from "@/shared/api/tauri";

export type ExistingAgentRegistrationResult = {
  agentPubkey: string;
  displayName: string;
  publicationStatus: "published" | "queued";
  alreadyRegistered: boolean;
  relayMessage: string | null;
};

type RawExistingAgentRegistrationResult = {
  agentPubkey: string;
  displayName: string;
  publicationStatus: "published" | "queued";
  alreadyRegistered: boolean;
  relayMessage?: string;
};

export async function registerExistingAgent(
  agentPubkey: string,
): Promise<ExistingAgentRegistrationResult> {
  const result = await invokeTauri<RawExistingAgentRegistrationResult>(
    "register_existing_agent",
    { input: { agentPubkey } },
  );
  return {
    ...result,
    relayMessage: result.relayMessage ?? null,
  };
}
