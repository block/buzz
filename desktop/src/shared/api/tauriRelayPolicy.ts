import { invokeTauri } from "@/shared/api/tauri";

export type RelayConnectionPolicy = {
  lockedToDefaultRelay: boolean;
  defaultRelayUrl: string;
};

export function getRelayConnectionPolicy(): Promise<RelayConnectionPolicy> {
  return invokeTauri<RelayConnectionPolicy>("get_relay_connection_policy");
}

export function assertRelayUrlAllowed(relayUrl: string): Promise<void> {
  return invokeTauri("assert_relay_url_allowed", { relayUrl });
}
