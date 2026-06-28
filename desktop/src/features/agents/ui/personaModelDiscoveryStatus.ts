import type { AgentModelsResponse } from "@/shared/api/types";

export type PersonaModelDiscoveryStatus = {
  message: string;
  tone: "muted" | "warning";
};

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

function knownProviderLabel(provider: string): string | null {
  switch (provider.trim()) {
    case "anthropic":
      return "Anthropic";
    case "databricks":
      return "Databricks";
    case "openai":
      return "OpenAI";
    case "openai-compat":
      return "OpenAI-compatible";
    default:
      return null;
  }
}

function providerObjectLabel(provider: string): string {
  return knownProviderLabel(provider) ?? "the selected provider";
}

function providerSubjectLabel(provider: string): string {
  return knownProviderLabel(provider) ?? "The selected provider";
}

export function formatModelDiscoveryErrorStatus(
  error: unknown,
  provider: string,
): PersonaModelDiscoveryStatus {
  const message = errorMessage(error);

  if (message.includes("ANTHROPIC_API_KEY required")) {
    return {
      message:
        "Using built-in model options. Add ANTHROPIC_API_KEY in Advanced env vars to load Anthropic models.",
      tone: "warning",
    };
  }

  if (message.includes("OPENAI_COMPAT_API_KEY required")) {
    return {
      message:
        "Using built-in model options. Add OPENAI_COMPAT_API_KEY in Advanced env vars to load OpenAI models.",
      tone: "warning",
    };
  }

  if (
    message.includes("DATABRICKS_HOST required") ||
    message.includes("DATABRICKS_MODEL required")
  ) {
    return {
      message:
        "Using built-in Databricks model options. DATABRICKS_HOST and DATABRICKS_MODEL are required for live Databricks models.",
      tone: "warning",
    };
  }

  if (message.includes("BUZZ_AGENT_PROVIDER required")) {
    return {
      message:
        "Using built-in model options. Select an LLM provider or set DATABRICKS_HOST and DATABRICKS_MODEL to load live models.",
      tone: "warning",
    };
  }

  return {
    message: `Using built-in model options. Could not load live models for ${providerObjectLabel(
      provider,
    )}.`,
    tone: "warning",
  };
}

export function formatModelDiscoveryFallbackStatus({
  provider,
  response,
}: {
  provider: string;
  response: AgentModelsResponse | null;
}): PersonaModelDiscoveryStatus | null {
  if (!response || response.models.length > 0) {
    return null;
  }

  if (!response.supportsSwitching) {
    return {
      message: `Using built-in model options. ${providerSubjectLabel(
        provider,
      )} does not expose a live model list yet.`,
      tone: "muted",
    };
  }

  return {
    message: "Using built-in model options. No live models were reported.",
    tone: "muted",
  };
}
