import { invokeTauri } from "@/shared/api/tauri";

/** Account-scoped agent identity learned from kind:30177 without local keys. */
export type ManagedAgentReference = {
  pubkey: string;
  name: string;
  personaId: string;
};

type RawManagedAgentReference = {
  pubkey: string;
  name: string;
  persona_id: string;
};

export async function listManagedAgentReferences(): Promise<
  ManagedAgentReference[]
> {
  return (
    await invokeTauri<RawManagedAgentReference[]>(
      "list_managed_agent_references",
    )
  ).map((reference) => ({
    pubkey: reference.pubkey,
    name: reference.name,
    personaId: reference.persona_id,
  }));
}
