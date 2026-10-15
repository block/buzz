export type PersonaModelDiscoveryStatus = {
  message: string;
  tone: "muted" | "warning";
};

/**
 * Availability reasons the ACP runtime catalog reports for a harness that
 * cannot run. `available` is excluded: it has no status to explain.
 */
export type UnavailableRuntimeAvailability =
  | "adapter_missing"
  | "adapter_outdated"
  | "cli_missing"
  | "not_installed";

/**
 * Explains a harness that the catalog already knows is unavailable.
 *
 * This is deliberately not routed through `formatModelDiscoveryErrorStatus`:
 * that formatter interprets *live catalog fetch* failures, so an availability
 * reason falls through to its generic "could not load live models for
 * <provider>" fallback and blames the provider for a missing local adapter or
 * CLI. The harness dropdown already names the reason via
 * `formatRuntimeOptionLabel`; this keeps the model status line consistent with
 * it.
 */
export function formatRuntimeAvailabilityStatus(
  availability: UnavailableRuntimeAvailability,
  agentLabel?: string,
): PersonaModelDiscoveryStatus {
  const harness = agentLabel?.trim() ? agentLabel.trim() : "This harness";
  const reason =
    availability === "cli_missing"
      ? `${harness} is installed but its CLI was not found. Install the CLI and sign in, then reopen this dialog.`
      : availability === "not_installed"
        ? `${harness} is not installed. Install it, then reopen this dialog.`
        : availability === "adapter_missing"
          ? `${harness} is missing its ACP adapter. Install the adapter, then reopen this dialog.`
          : `${harness}'s ACP adapter is out of date. Update the adapter, then reopen this dialog.`;
  return {
    message: `Using built-in model options. ${reason}`,
    tone: "warning",
  };
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

export function formatModelDiscoveryErrorStatus(
  error: unknown,
  provider: string,
  agentLabel?: string,
): PersonaModelDiscoveryStatus | null {
  const message = errorMessage(error);

  if (provider.trim() === "relay-mesh") {
    if (message.includes("waiting for the current member roster")) {
      return {
        message:
          "Buzz is waiting for the relay's member roster. Try again shortly; if this persists, check the relay's membership configuration.",
        tone: "warning",
      };
    }

    if (isEmptySharedComputeError(message)) {
      return {
        message:
          "No members are sharing compute right now. On a member machine, open Settings > Compute, choose a model, and turn on Share this machine.",
        tone: "warning",
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
      };
    }

    return {
      message:
        "Buzz couldn't check shared compute through the relay. Check your relay connection and try again.",
      tone: "warning",
    };
  }

  // Spec-reserved auth error text (agent-client-protocol ErrorCode::AuthRequired),
  // surfaced verbatim through buzz-acp's stderr — generic across conformant
  // harnesses (e.g. cursor-agent when not signed in). Match the message text,
  // NOT code -32000: that code is also the catch-all fallback for unclassified
  // errors, so matching it would swallow unrelated failures into "sign in".
  if (message.toLowerCase().includes("authentication required")) {
    const label = agentLabel?.trim();
    return {
      message: `${label || "This agent"} requires sign-in before models can load. Sign in with the ${label || "agent's"} CLI in a terminal, then try again.`,
      tone: "warning",
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
      message:
        "Enter an OpenAI runtime API key (OPENAI_COMPAT_API_KEY) to load OpenAI models.",
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

  // Databricks transparent auth (agent_models_databricks.rs). The backend
  // launches the browser OAuth flow itself from every discovery surface, so
  // these are terminal outcomes the user should see, not raw error text.
  // Matched on the stable error strings the backend emits (string matching is
  // this file's convention until typed error codes arrive).
  const databricksStatus = formatDatabricksAuthStatus(message);
  if (databricksStatus !== null) {
    return databricksStatus;
  }

  return {
    message: `Using built-in model options. Could not load live models for ${providerObjectLabel(
      provider,
    )}.`,
    tone: "warning",
  };
}

/**
 * Maps the terminal Databricks sign-in states to user-facing guidance, or null
 * when the error is not a Databricks sign-in outcome. "Sign-in required" is a
 * quiet muted note (a passive surface hit its cooldown, or an unsaved draft
 * can't launch the browser); a failed, cancelled, or timed-out sign-in is a
 * warning that points the user at the explicit retry path.
 */
function formatDatabricksAuthStatus(
  message: string,
): PersonaModelDiscoveryStatus | null {
  if (message.includes("Databricks sign-in is required")) {
    return {
      message:
        "Databricks sign-in is required. Open the model picker to sign in, or run `buzz-agent auth databricks` in a terminal.",
      tone: "muted",
    };
  }

  if (
    message.includes("Databricks sign-in failed") ||
    message.includes("Databricks sign-in timed out")
  ) {
    return {
      message:
        "Databricks sign-in didn't complete. Open the model picker to retry, or run `buzz-agent auth databricks` in a terminal.",
      tone: "warning",
    };
  }

  return null;
}
