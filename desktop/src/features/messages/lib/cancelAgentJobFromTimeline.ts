import { cancelAgentJob } from "@/shared/api/agentJobs";
import type { AgentJobView } from "./agentJobProjection";

export function cancelAgentJobFromTimeline(job: AgentJobView): void {
  void cancelAgentJob(job).catch((error) => {
    console.error("Failed to cancel managed agent job", error);
  });
}
