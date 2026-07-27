export type ManagedAgentRuntimeCapabilities = {
  runtimeIconUrl: string | null;
  runtimeAvatarUrl: string | null;
  runtimeSupersededAvatarUrls: string[];
  supportsBuzzModelConfig: boolean | null;
};

export type RuntimeCatalogCapabilities = {
  displayLabel: string;
  sortPriority: number;
  onboardingVisible: boolean;
  iconUrl: string;
  iconScale: number;
  supersededAvatarUrls: string[];
  supportsBuzzModelConfig: boolean;
};

export type RuntimeConfigCapabilities = {
  supportsBuzzModelConfig: boolean | null;
};

export type ManagedAgentLog = {
  content: string;
  logPath: string;
};

export type CancelManagedAgentTurnResult = {
  status: "sent" | "no_active_turn";
};

/**
 * Outcome of a live model-switch control frame, surfaced asynchronously via
 * the agent's control-result observer frame.
 */
export type SwitchManagedAgentModelStatus =
  | "sent"
  | "turn_ending"
  | "switched"
  | "unsupported_model"
  | "no_active_turn";

export type ControlResultFrame = {
  type: "cancel_turn" | "switch_model" | "permission_decision";
  status: string;
  modelId?: string;
};
