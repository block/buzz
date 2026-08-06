export type PersonaModelDiscoveryStatus = {
  message: string;
  tone: "muted" | "warning";
  /** Optional documentation link rendered beneath the message. */
  link?: { href: string; label: string };
};

/**
 * Sentinel prefix emitted by the Tauri model-discovery commands when a
 * runtime's ACP adapter binary is not installed. Mirrors
 * `ADAPTER_MISSING_PREFIX` in `managed_agents/discovery.rs`; the JSON payload
 * carries that runtime's install hint, commands, and documentation URL so this
 * module renders actionable guidance without a second copy of the catalog.
 */
const ADAPTER_MISSING_PREFIX = "ADAPTER_MISSING:";

type AdapterMissingPayload = {
  runtimeLabel: string;
  hint: string;
  commands: string[];
  url: string;
};

function parseAdapterMissingPayload(
  message: string,
): AdapterMissingPayload | null {
  const index = message.indexOf(ADAPTER_MISSING_PREFIX);
  if (index < 0) {
    return null;
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(message.slice(index + ADAPTER_MISSING_PREFIX.length));
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) {
    return null;
  }

  const raw = parsed as Record<string, unknown>;
  const commands = Array.isArray(raw.commands)
    ? raw.commands.filter((value): value is string => typeof value === "string")
    : [];
  return {
    runtimeLabel:
      typeof raw.runtimeLabel === "string" ? raw.runtimeLabel : "This agent",
    hint: typeof raw.hint === "string" ? raw.hint : "",
    commands,
    url: typeof raw.url === "string" ? raw.url : "",
  };
}

function adapterMissingStatus(
  payload: AdapterMissingPayload,
): PersonaModelDiscoveryStatus {
  // The catalog hint already embeds the install command verbatim, so it is used
  // as-is. Runtimes that ship commands but no hint get a synthesized sentence.
  const message =
    payload.hint.trim() ||
    (payload.commands.length > 0
      ? `${payload.runtimeLabel} needs an ACP adapter before models can load. Install it with: ${payload.commands.join(" && ")}.`
      : `${payload.runtimeLabel} needs an ACP adapter before models can load.`);

  return {
    message,
    tone: "warning",
    ...(payload.url
      ? { link: { href: payload.url, label: "Installation instructions" } }
      : {}),
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

  // Checked before every provider branch: a missing adapter is a local install
  // problem, and its remedy is the same whatever provider the form selected.
  const adapterMissing = parseAdapterMissingPayload(message);
  if (adapterMissing) {
    return adapterMissingStatus(adapterMissing);
  }

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

  return {
    message: `Using built-in model options. Could not load live models for ${providerObjectLabel(
      provider,
    )}.`,
    tone: "warning",
  };
}
