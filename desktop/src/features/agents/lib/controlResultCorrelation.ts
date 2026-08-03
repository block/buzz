import type { ControlResultFrame } from "@/shared/api/types";

const DEFERRED_MODEL_RESULTS = new Set([
  "switched",
  "switch_failed",
  "unsupported_model",
]);

function validTurnId(value: unknown): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= 128;
}

/**
 * Correlate a control result to the turn originally targeted by Desktop.
 * Deferred model results are emitted from the replacement turn, so their
 * payload's original turn id intentionally differs from the observer envelope.
 * Immediate results must agree with the envelope when both ids are present.
 */
export function correlateControlResultFrame(
  payload: ControlResultFrame,
  envelopeTurnId: string | null,
): ControlResultFrame | null {
  const payloadTurnId = validTurnId(payload.turnId)
    ? payload.turnId
    : undefined;
  const envelopeId = validTurnId(envelopeTurnId) ? envelopeTurnId : undefined;

  if (
    payloadTurnId &&
    envelopeId &&
    payloadTurnId !== envelopeId &&
    !DEFERRED_MODEL_RESULTS.has(payload.status)
  ) {
    return null;
  }

  const turnId = payloadTurnId ?? envelopeId;
  return turnId ? { ...payload, turnId } : payload;
}
