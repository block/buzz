import { invokeTauri } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";

export function getProjectRevisionHeads(
  coordinates: string[],
): Promise<RelayEvent[]> {
  return invokeTauri<RelayEvent[]>("get_project_revision_heads", {
    coordinates,
  });
}
