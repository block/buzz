/**
 * Outcome of a live `switch_model` control frame, surfaced asynchronously via
 * the agent's `control_result` observer frame. Busy path: `sent` (cancel +
 * requeue on the new model) or `turn_ending` (oneshot already consumed this
 * turn). Idle path: `switched`, `unsupported_model`, or `no_active_turn`.
 */
export type SwitchManagedAgentModelStatus =
  | "sent"
  | "turn_ending"
  | "switched"
  | "unsupported_model"
  | "superseded"
  | "no_active_turn";

export type ControlResultFrame = {
  type: "cancel_turn" | "switch_model";
  status: string;
  modelId?: string;
  channelId?: string;
  generation?: number;
  requestId?: string | null;
};
