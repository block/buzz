/**
 * Outcome of a live `switch_model` control frame, surfaced asynchronously via
 * the agent's `control_result` observer frame. Busy path: `sent` (cancel +
 * requeue on the new model) or `turn_ending` (oneshot already consumed this
 * turn). Idle-race path can report `recycling`. Only `switched` proves that
 * the requested model reached a fresh channel session; `switch_failed`,
 * `unsupported_model`, and `no_active_turn` are terminal failures.
 */
export type SwitchManagedAgentModelStatus =
  | "sent"
  | "turn_ending"
  | "recycling"
  | "switched"
  | "switch_failed"
  | "unsupported_model"
  | "no_active_turn";

export type ControlResultFrame = {
  type: "cancel_turn" | "switch_model";
  status: string;
  /** Exact desktop-generated identifier echoed by every switch result. */
  requestId?: string;
  modelId?: string;
  channelId?: string;
  /** ID of the signed kind-24200 relay event carrying this result. */
  relayEventId?: string;
  /** Signed Nostr `created_at` for the carrying relay event. */
  relayCreatedAt?: number;
  /** Timestamp inside the encrypted, signed observer event payload. */
  observerTimestamp?: string;
  /** Process-local sequence inside the encrypted, signed observer payload. */
  observerSeq?: number;
};
