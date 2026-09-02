import { invokeTauri } from "@/shared/api/tauri";

export type HermesProfileDistribution = {
  name: string | null;
  version: string | null;
  source: string | null;
};

export type HermesProfileInfo = {
  name: string;
  displayName: string;
  description: string;
  descriptionAuto: boolean;
  isDefault: boolean;
  active: boolean;
  model: string | null;
  provider: string | null;
  gatewayRunning: boolean;
  alias: string | null;
  distribution: HermesProfileDistribution | null;
};

export type HermesProfileInventory = {
  activeProfile: string;
  profiles: HermesProfileInfo[];
};

/** Discover the installed Hermes profile inventory through its versioned JSON CLI contract. */
export function discoverHermesProfiles(): Promise<HermesProfileInventory> {
  return invokeTauri<HermesProfileInventory>("discover_hermes_profiles");
}
