const AGENT_DEFAULTS_SAVE_FALLBACK = "Couldn't save.";

/** Preserve actionable backend validation copy across the Tauri error boundary. */
export function agentDefaultsSaveErrorMessage(error: unknown): string {
  const message =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : "";
  return message.trim() || AGENT_DEFAULTS_SAVE_FALLBACK;
}
