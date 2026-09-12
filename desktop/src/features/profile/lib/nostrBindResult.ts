const RESULT_PROTOCOL = "run402_adoption_result_v1";
const DEFAULT_POLL_INTERVAL_MS = 1_000;
const DEFAULT_MAX_OBSERVATION_MS = 5 * 60_000;
const EXPIRY_GRACE_MS = 15_000;

const ACTION_REQUIRED_CODES = new Set([
  "BUZZ_ADOPTION_CALLBACK_INVALID",
  "BUZZ_ADOPTION_OFFER_NOT_ACTIVE",
  "BUZZ_ADOPTION_TARGET_MISMATCH",
  "BUZZ_IDEMPOTENCY_CONFLICT",
  "BUZZ_OWNER_PROOF_INVALID",
  "STEP_UP_REQUIRED",
  "BUZZ_ADOPTION_COMPLETION_REJECTED",
] as const);

export type NostrBindActionRequiredCode =
  | "BUZZ_ADOPTION_CALLBACK_INVALID"
  | "BUZZ_ADOPTION_OFFER_NOT_ACTIVE"
  | "BUZZ_ADOPTION_TARGET_MISMATCH"
  | "BUZZ_IDEMPOTENCY_CONFLICT"
  | "BUZZ_OWNER_PROOF_INVALID"
  | "STEP_UP_REQUIRED"
  | "BUZZ_ADOPTION_COMPLETION_REJECTED";

export type NostrBindResultState =
  | { status: "waiting" }
  | { status: "completed" }
  | {
      status: "action_required";
      resultCode: NostrBindActionRequiredCode;
    }
  | { status: "expired" }
  | { status: "cancelled" }
  | {
      status: "unable_to_confirm";
      reason:
        | "no_final_result"
        | "unreachable"
        | "invalid_response"
        | "unsafe_endpoint";
    };

export type NostrBindResultObservation =
  | NostrBindResultState
  | { status: "superseded" };

export type NostrBindResultPresentation = {
  title: string;
  description: string;
  nextAction: string;
  closeLabel: string;
  tone: "neutral" | "success" | "warning";
};

type NostrBindResultExtension = {
  resultProtocol?: string;
  resultUrl?: string;
};

type ObserveNostrBindResultOptions = {
  resultUrl: string;
  ownerProofEvent: unknown;
  expiresAt: string;
  readResult: (input: {
    resultUrl: string;
    ownerProofEvent: unknown;
  }) => Promise<unknown>;
  isActive: () => boolean;
  onState: (state: NostrBindResultState) => void;
  now?: () => number;
  sleep?: (milliseconds: number) => Promise<void>;
  pollIntervalMs?: number;
  maxObservationMs?: number;
};

export function hasNostrBindResultAcknowledgement(
  payload: NostrBindResultExtension,
): payload is NostrBindResultExtension & {
  resultProtocol: typeof RESULT_PROTOCOL;
  resultUrl: string;
} {
  return (
    payload.resultProtocol === RESULT_PROTOCOL &&
    typeof payload.resultUrl === "string" &&
    payload.resultUrl.length > 0
  );
}

const ACTION_REQUIRED_COPY: Record<
  NostrBindActionRequiredCode,
  Pick<NostrBindResultPresentation, "description" | "nextAction">
> = {
  BUZZ_ADOPTION_CALLBACK_INVALID: {
    description:
      "Run402 could not verify the browser return for this approval.",
    nextAction:
      "Return to the original Run402 tab and start a fresh approval. Keep that new tab open until Buzz confirms it.",
  },
  BUZZ_ADOPTION_OFFER_NOT_ACTIVE: {
    description: "The Run402 ownership invitation is no longer active.",
    nextAction:
      "Ask the agent that created the deployment to create a new ownership invitation, then use its new Buzz link.",
  },
  BUZZ_ADOPTION_TARGET_MISMATCH: {
    description:
      "This approval does not match the Run402 organization that requested it.",
    nextAction:
      "Return to the original Run402 tab and start again from the invitation for the intended organization.",
  },
  BUZZ_IDEMPOTENCY_CONFLICT: {
    description:
      "Run402 found a different request already using this approval attempt.",
    nextAction:
      "Return to the original Run402 tab and start a new approval. Do not reuse the old Buzz link.",
  },
  BUZZ_OWNER_PROOF_INVALID: {
    description:
      "Run402 could not match this signature to the linked Buzz owner.",
    nextAction:
      "Make sure Buzz is using the identity named in Run402, then start a fresh approval from the original Run402 tab.",
  },
  STEP_UP_REQUIRED: {
    description:
      "Run402 needs a fresh security check before it can add you as a co-owner.",
    nextAction:
      "Return to the original Run402 tab, complete its passkey check, and then start a fresh Buzz approval if requested.",
  },
  BUZZ_ADOPTION_COMPLETION_REJECTED: {
    description: "Run402 did not complete this co-ownership request.",
    nextAction:
      "Return to the original Run402 tab for the specific recovery step. Start a fresh approval only if Run402 asks you to.",
  },
};

export function getNostrBindResultPresentation(
  state: NostrBindResultState,
): NostrBindResultPresentation {
  switch (state.status) {
    case "waiting":
      return {
        title: "Waiting for Run402 confirmation",
        description:
          "Buzz sent your signed approval to the browser. Run402 is now checking it and adding you as a co-owner.",
        nextAction:
          "Keep this window open. If you close it, the handoff may still finish in Run402, but Buzz will stop checking automatically.",
        closeLabel: "Stop waiting and close",
        tone: "neutral",
      };
    case "completed":
      return {
        title: "You’re now a Run402 co-owner",
        description:
          "Run402 confirmed this exact Buzz approval and completed the ownership handoff.",
        nextAction: "This window will close automatically.",
        closeLabel: "Close now",
        tone: "success",
      };
    case "action_required": {
      const copy = ACTION_REQUIRED_COPY[state.resultCode];
      return {
        title: "Run402 needs another step",
        ...copy,
        closeLabel: "Close and return to Run402",
        tone: "warning",
      };
    }
    case "expired":
      return {
        title: "This approval expired",
        description:
          "Run402 did not complete the ownership handoff before this one-time approval expired.",
        nextAction:
          "Return to the original Run402 tab and start a fresh approval. The expired approval cannot be reused.",
        closeLabel: "Close and return to Run402",
        tone: "warning",
      };
    case "cancelled":
      return {
        title: "This handoff was cancelled",
        description:
          "Run402 cancelled this ownership attempt. No completion was confirmed.",
        nextAction:
          "Return to the original Run402 tab. Start a new approval only if you still want to become a co-owner.",
        closeLabel: "Close and return to Run402",
        tone: "warning",
      };
    case "unable_to_confirm": {
      const reasonCopy = {
        no_final_result:
          "Run402 did not return a final result before the confirmation window ended.",
        unreachable:
          "Buzz could not reach Run402 before the confirmation window ended.",
        invalid_response:
          "Run402 returned a response Buzz could not safely recognize.",
        unsafe_endpoint:
          "Buzz refused to contact an unsafe confirmation endpoint.",
      }[state.reason];
      return {
        title: "Couldn’t confirm the Run402 result",
        description: `${reasonCopy} This does not prove that ownership succeeded or failed.`,
        nextAction:
          "Check the original Run402 tab for the authoritative status before starting another approval.",
        closeLabel: "Close and check Run402",
        tone: "warning",
      };
    }
  }
}

function isExactKeySet(
  value: Record<string, unknown>,
  keys: string[],
): boolean {
  const actual = Object.keys(value).sort();
  return (
    actual.length === keys.length &&
    actual.every((key, index) => key === keys[index])
  );
}

function parseNativeResult(value: unknown): NostrBindResultState | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  if (record.status === "action_required") {
    if (
      !isExactKeySet(record, ["resultCode", "status"]) ||
      typeof record.resultCode !== "string" ||
      !ACTION_REQUIRED_CODES.has(
        record.resultCode as NostrBindActionRequiredCode,
      )
    ) {
      return null;
    }
    return {
      status: "action_required",
      resultCode: record.resultCode as NostrBindActionRequiredCode,
    };
  }
  if (
    !isExactKeySet(record, ["status"]) ||
    !["pending", "completed", "expired", "cancelled"].includes(
      record.status as string,
    )
  ) {
    return null;
  }
  if (record.status === "pending") {
    return { status: "waiting" };
  }
  return record as
    | { status: "completed" }
    | { status: "expired" }
    | { status: "cancelled" };
}

function resultErrorReason(
  error: unknown,
): "unreachable" | "invalid_response" | "unsafe_endpoint" {
  if (typeof error === "object" && error !== null && "kind" in error) {
    const kind = (error as { kind?: unknown }).kind;
    if (kind === "invalid_response" || kind === "unsafe_endpoint") {
      return kind;
    }
  }
  return "unreachable";
}

function defaultSleep(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

/**
 * Observe one signed approval until its issuer reports a terminal state.
 * The caller owns active-request identity; a superseded request never emits a
 * terminal state, even if its in-flight native read later returns completed.
 */
export async function observeNostrBindResult(
  options: ObserveNostrBindResultOptions,
): Promise<NostrBindResultObservation> {
  const now = options.now ?? Date.now;
  const sleep = options.sleep ?? defaultSleep;
  const pollIntervalMs = options.pollIntervalMs ?? DEFAULT_POLL_INTERVAL_MS;
  const maxObservationMs =
    options.maxObservationMs ?? DEFAULT_MAX_OBSERVATION_MS;
  const startedAt = now();
  const expiresAt = Date.parse(options.expiresAt);
  if (!Number.isFinite(expiresAt)) {
    return {
      status: "unable_to_confirm",
      reason: "invalid_response",
    };
  }
  const deadline = Math.min(
    startedAt + maxObservationMs,
    expiresAt + EXPIRY_GRACE_MS,
  );
  let lastTransientReason: "unreachable" | null = null;

  if (!options.isActive()) {
    return { status: "superseded" };
  }
  options.onState({ status: "waiting" });

  while (options.isActive()) {
    try {
      const value = await options.readResult({
        resultUrl: options.resultUrl,
        ownerProofEvent: options.ownerProofEvent,
      });
      if (!options.isActive()) {
        return { status: "superseded" };
      }
      const state = parseNativeResult(value);
      if (!state) {
        const invalid = {
          status: "unable_to_confirm",
          reason: "invalid_response",
        } as const;
        options.onState(invalid);
        return invalid;
      }
      if (state.status !== "waiting") {
        options.onState(state);
        return state;
      }
      lastTransientReason = null;
    } catch (error) {
      if (!options.isActive()) {
        return { status: "superseded" };
      }
      const reason = resultErrorReason(error);
      if (reason !== "unreachable") {
        const unable = { status: "unable_to_confirm", reason } as const;
        options.onState(unable);
        return unable;
      }
      lastTransientReason = reason;
    }

    const remainingMs = deadline - now();
    if (remainingMs <= 0) {
      const unable = {
        status: "unable_to_confirm",
        reason: lastTransientReason ?? "no_final_result",
      } as const;
      options.onState(unable);
      return unable;
    }
    await sleep(Math.min(pollIntervalMs, remainingMs));
    if (!options.isActive()) {
      return { status: "superseded" };
    }
    if (now() >= deadline) {
      const unable = {
        status: "unable_to_confirm",
        reason: lastTransientReason ?? "no_final_result",
      } as const;
      options.onState(unable);
      return unable;
    }
  }

  return { status: "superseded" };
}
