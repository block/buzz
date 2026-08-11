import { invokeTauri } from "@/shared/api/tauri";

export type ThreadParticipationPref = {
  enabled: boolean;
};

export async function getThreadParticipation(): Promise<ThreadParticipationPref> {
  return invokeTauri<ThreadParticipationPref>("get_thread_participation");
}

export async function setThreadParticipation(
  enabled: boolean,
): Promise<ThreadParticipationPref> {
  return invokeTauri<ThreadParticipationPref>("set_thread_participation", {
    enabled,
  });
}
