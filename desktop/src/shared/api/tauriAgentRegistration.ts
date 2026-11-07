import { invokeTauri } from "@/shared/api/tauri";
import type { RespondToMode } from "@/shared/api/types";

export type RegisterExistingAgentInput = {
  agentPubkey: string;
  respondTo: RespondToMode;
  respondToAllowlist: string[];
};

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
  input: RegisterExistingAgentInput,
): Promise<ExistingAgentRegistrationResult> {
  const result = await invokeTauri<RawExistingAgentRegistrationResult>(
    "register_existing_agent",
    { input },
  );
  return {
    ...result,
    relayMessage: result.relayMessage ?? null,
  };
}
