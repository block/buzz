import { invokeTauri } from "@/shared/api/tauri";

/** Clears community-scoped agent activity from the native tray menu. */
export function clearTrayAgentActivity(): Promise<void> {
  return invokeTauri("clear_tray_agent_activity");
}
