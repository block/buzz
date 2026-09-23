/**
 * Permission policy controlling how the ACP harness answers
 * `session/request_permission` calls.
 *
 * - `ask`: Show an actionable Allow/Deny card in the transcript (desktop default).
 * - `allow`: Auto-approve the unique `allow_once` option (explicit opt-in).
 * - `reject`: Auto-deny all requests without surfacing a card.
 */
export type PermissionPolicy = "ask" | "allow" | "reject";

/**
 * Where the effective permission policy value came from.
 *
 * - `agent`: Per-agent override set on this specific agent record.
 * - `definition`: Inherited from the linked persona definition's default policy.
 * - `global_default`: Fleet-wide default from the global agent config.
 * - `built_in`: Neither layer had a value; the desktop built-in default (`ask`) applies.
 */
export type PermissionPolicySource =
  | "agent"
  | "definition"
  | "global_default"
  | "built_in";

export type CancelManagedAgentTurnResult = {
  status: "sent" | "no_active_turn";
};

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
  | "no_active_turn"
  | "failure";

export type ControlResultFrame = {
  type: "cancel_turn" | "switch_model" | "permission_decision";
  status: string;
  modelId?: string;
  /** Present on `permission_decision` results — identifies the request card to retire. */
  requestNonce?: string;
  /** Opaque per-pick id echoed from the request; correlates late frames. */
  requestId?: string;
  /** Buzz channel UUID from the observer envelope; disambiguates channels. */
  channelId?: string | null;
};

export type BackendProviderCandidate = {
  id: string;
  binaryPath: string;
};
