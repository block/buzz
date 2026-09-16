import { invokeTauri } from "@/shared/api/tauri";

export const getAgentAccessOwnerOnly = () =>
  invokeTauri<boolean>("agent_access_owner_only");
