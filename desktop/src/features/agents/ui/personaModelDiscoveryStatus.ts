export type PersonaModelDiscoveryStatus = {
  message: string;
  tone: "muted" | "warning";
  /**
   * When true, the model field should offer a Retry control that re-runs
   * discovery (timeouts, empty catalogs, path failures). Credential-missing
   * states stay non-retryable — the user needs to fill a field first.
   */
  retryable?: boolean;
};

/**
 * After this many ms of discovery, surface the long status-line note under
 * the Model field. Until then the control alone shows
 * {@link MODEL_DISCOVERY_LOADING_SHORT} — no duplicate under-field copy.
 * See #2261.
 */
export const MODEL_DISCOVERY_SLOW_MS = 10_000;

/**
 * Short label for the model **control** (closed trigger / disabled option).
 * Never put the long progressive sentence in the control — it truncates.
 */
export const MODEL_DISCOVERY_LOADING_SHORT = "Loading models…";

/**
 * Long status-line copy for a slow model probe, or `null` before the
 * slow phase so the under-field line stays empty (control already shows
 * {@link MODEL_DISCOVERY_LOADING_SHORT}).
 *
 * @param slow - true once discovery has been in flight ≥ {@link MODEL_DISCOVERY_SLOW_MS}
 */
export function formatModelDiscoveryLoadingMessage(
  slow: boolean,
): string | null {
  if (!slow) return null;
  return "Still loading models… first launch of some harnesses (especially Codex) can take 20–60 seconds.";
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  try {
    return JSON.stringify(error);
  } catch {
    return "Unknown model discovery error";
  }
}

function providerObjectLabel(provider: string): string {
  switch (provider.trim()) {
    case "anthropic":
      return "Anthropic";
    case "openai":
      return "OpenAI";
    case "openai-compat":
      return "OpenAI-compatible";
    default:
      return provider.trim() || "this provider";
  }
}

function isEmptySharedComputeError(message: string): boolean {
  const normalized = message.toLowerCase();
  return (
    normalized.includes("shared compute status is not published") ||
    normalized.includes("no buzz shared compute serving members") ||
    normalized.includes("no live buzz shared compute models") ||
    normalized.includes("no live member is serving") ||
    normalized.includes("requires a live serving member")
  );
}

/** True when stderr/IPC text indicates the ACP models probe hit its budget. */
export function isModelDiscoveryTimeoutError(message: string): boolean {
  const normalized = message.toLowerCase();
  // buzz-acp: `error: agent timed out (10s)` / `(45s)`
  // desktop: `buzz-acp models failed (exit N): ... timed out ...`
  if (normalized.includes("timed out")) return true;
  if (normalized.includes("timeout") && normalized.includes("agent")) {
    return true;
  }
  return false;
}

function isProgramNotFoundError(message: string): boolean {
  const normalized = message.toLowerCase();
  return (
    normalized.includes("program not found") ||
    normalized.includes("the system cannot find the file") ||
    normalized.includes("enoent") ||
    (normalized.includes("not found") &&
      (normalized.includes("codex") ||
        normalized.includes("claude") ||
        normalized.includes("agent")))
  );
}

export function formatModelDiscoveryErrorStatus(
  error: unknown,
  provider: string,
): PersonaModelDiscoveryStatus | null {
  const message = errorMessage(error);

  if (provider.trim() === "relay-mesh") {
    if (message.includes("waiting for the current member roster")) {
      return {
        message:
          "Buzz is waiting for the relay's member roster. Try again shortly; if this persists, check the relay's membership configuration.",
        tone: "warning",
        retryable: true,
      };
    }

    if (isEmptySharedComputeError(message)) {
      return {
        message:
          "No members are sharing compute right now. On a member machine, open Settings > Compute, choose a model, and turn on Share this machine.",
        tone: "warning",
        retryable: true,
      };
    }

    if (message.includes("shared compute is not available in this build")) {
      return {
        message:
          "This version of Buzz cannot use shared compute. Update Buzz or choose another provider.",
        tone: "warning",
      };
    }

    if (message.includes("shared compute status is malformed")) {
      return {
        message:
          "Buzz received an invalid shared compute status. Check the member machine, then try again.",
        tone: "warning",
        retryable: true,
      };
    }

    return {
      message:
        "Buzz couldn't check shared compute through the relay. Check your relay connection and try again.",
      tone: "warning",
      retryable: true,
    };
  }

  if (message.includes("ANTHROPIC_API_KEY required")) {
    return {
      message: "Enter an Anthropic API key to load Anthropic models.",
      tone: "warning",
    };
  }

  if (message.includes("OPENAI_COMPAT_API_KEY required")) {
    return {
      message: "Enter an OpenAI API key to load OpenAI models.",
      tone: "warning",
    };
  }

  if (
    message.includes("DATABRICKS_HOST required") ||
    message.includes("DATABRICKS_MODEL required") ||
    message.includes("BUZZ_AGENT_PROVIDER is required")
  ) {
    return null;
  }

  if (isModelDiscoveryTimeoutError(message)) {
    return {
      message:
        "Model discovery timed out. Codex and some other harnesses can take 20–60 seconds to start the first time — retry, or wait a moment and try again.",
      tone: "warning",
      retryable: true,
    };
  }

  if (isProgramNotFoundError(message)) {
    return {
      message:
        "Could not find the agent harness on PATH. Install or reinstall it, ensure its install directory is on PATH, then retry.",
      tone: "warning",
      retryable: true,
    };
  }

  return {
    message: `Using built-in model options. Could not load live models for ${providerObjectLabel(
      provider,
    )}.`,
    tone: "warning",
    retryable: true,
  };
}
