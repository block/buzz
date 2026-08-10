import {
  useCreateManagedAgentMutation,
  useCreatePersonaMutation,
  useDeleteManagedAgentMutation,
  useDeletePersonaMutation,
  useSetManagedAgentStartOnAppLaunchMutation,
  useSetPersonaActiveMutation,
  useStartManagedAgentMutation,
  useStopManagedAgentMutation,
  useUpdateManagedAgentMutation,
  useUpdatePersonaMutation,
} from "@/features/agents/hooks";
import { useRedeployManagedAgentMutation } from "@/features/agents/useRedeployManagedAgentMutation";

export function useAgentProfileMutations() {
  const createAgentMutation = useCreateManagedAgentMutation();
  const updateManagedAgentMutation = useUpdateManagedAgentMutation();
  const startAgentMutation = useStartManagedAgentMutation();
  const redeployAgentMutation = useRedeployManagedAgentMutation();
  const stopAgentMutation = useStopManagedAgentMutation();
  const deleteAgentMutation = useDeleteManagedAgentMutation();
  const startOnLaunchMutation = useSetManagedAgentStartOnAppLaunchMutation();
  const createPersonaMutation = useCreatePersonaMutation();
  const updatePersonaMutation = useUpdatePersonaMutation();
  const deletePersonaMutation = useDeletePersonaMutation();
  const setPersonaActiveMutation = useSetPersonaActiveMutation();

  const isAgentActionPending =
    createAgentMutation.isPending ||
    updateManagedAgentMutation.isPending ||
    startAgentMutation.isPending ||
    redeployAgentMutation.isPending ||
    stopAgentMutation.isPending ||
    deleteAgentMutation.isPending ||
    startOnLaunchMutation.isPending ||
    createPersonaMutation.isPending ||
    updatePersonaMutation.isPending ||
    deletePersonaMutation.isPending ||
    setPersonaActiveMutation.isPending;

  return {
    createAgentMutation,
    updateManagedAgentMutation,
    startAgentMutation,
    redeployAgentMutation,
    stopAgentMutation,
    deleteAgentMutation,
    startOnLaunchMutation,
    createPersonaMutation,
    updatePersonaMutation,
    deletePersonaMutation,
    setPersonaActiveMutation,
    isAgentActionPending,
  };
}
