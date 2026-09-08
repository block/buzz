import type { ProjectActivitySummary, Repository } from "./hooks";
import type { RelayEvent } from "@/shared/api/types";

export function summarizeProjectActivityEvents(
  events: RelayEvent[],
  projects: Repository[],
): Record<string, ProjectActivitySummary>;
