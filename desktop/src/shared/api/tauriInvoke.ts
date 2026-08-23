import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import {
  activateRateLimit,
  parseRateLimitHint,
} from "@/shared/api/relayRateLimitGate";

/** Error normalized from a rejected Tauri invocation with its wire payload. */
export class TauriInvokeError extends Error {
  readonly payload: unknown;

  constructor(message: string, payload: unknown) {
    super(message);
    this.name = "TauriInvokeError";
    this.payload = payload;
  }
}

function toTauriError(error: unknown): Error {
  if (error instanceof Error) return error;
  if (typeof error === "string") return new TauriInvokeError(error, error);
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return new TauriInvokeError(error.message, error);
  }
  try {
    return new TauriInvokeError(JSON.stringify(error), error);
  } catch {
    return new TauriInvokeError("Unknown Tauri error", error);
  }
}

/** Apply a relay backoff when a Tauri error carries the relay 429 prefix. */
export function applyTauriRateLimitIfNeeded(message: string): void {
  if (message.startsWith("relay rate-limited:")) {
    activateRateLimit(parseRateLimitHint(message));
  }
}

/** Invoke a Tauri command with normalized errors and shared rate-limit handling. */
export async function invokeTauri<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (error) {
    const normalized = toTauriError(error);
    applyTauriRateLimitIfNeeded(normalized.message);
    throw normalized;
  }
}
