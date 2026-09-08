import { useQuery } from "@tanstack/react-query";
import { invokeTauri } from "@/shared/api/tauri";

export type DeviceAgentPolicy = {
  client_only: boolean;
  unique_names?: boolean;
  preferred_agents: {
    relay_url: string;
    owner_pubkey: string;
    name: string;
    pubkey: string;
    persona_id?: string | null;
  }[];
};

export type DeviceAgentPolicyStatus = {
  activeClientOnly: boolean;
  activeUniqueNames?: boolean;
  saved: DeviceAgentPolicy;
  restartRequired: boolean;
  loadError: string | null;
};

export const deviceAgentPolicyQueryKey = ["agent-device-policy"] as const;

export function useDeviceAgentPolicy() {
  return useQuery({
    queryKey: deviceAgentPolicyQueryKey,
    queryFn: () =>
      invokeTauri<DeviceAgentPolicyStatus>("get_agent_device_policy"),
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  });
}

export function saveDeviceAgentPolicy(policy: DeviceAgentPolicy) {
  return invokeTauri<DeviceAgentPolicyStatus>("set_agent_device_policy", {
    policy,
  });
}
