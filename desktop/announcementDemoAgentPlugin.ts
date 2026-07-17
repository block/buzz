import { loadEnv, type Plugin } from "vite";

const AGENT_RESPONSE_PATH = "/__announcement-demo/agent-response";
const MOCK_RELAY_QUERY_PATH = "/query";
const MAX_REQUEST_BYTES = 64 * 1024;
const REQUEST_TIMEOUT_MS = 45_000;

type AnnouncementDemoAgentMessage = {
  role: "assistant" | "user";
  content: string;
};

type AnnouncementDemoAgentRequest = {
  provider: "anthropic" | "openai";
  apiKey: string;
  model: string;
  systemPrompt: string;
  messages: AnnouncementDemoAgentMessage[];
};

type ProviderErrorBody = {
  error?: string | { message?: string };
  message?: string;
};

type OpenAiReasoningEffort = "low" | "minimal" | "none";

type AgentEnvironment = Record<string, string | undefined>;

function nonEmptyString(value: unknown) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function supportedProvider(value: unknown) {
  const provider = nonEmptyString(value)?.toLowerCase();
  return provider === "anthropic" || provider === "openai" ? provider : null;
}

function firstEnvironmentValue(
  environment: AgentEnvironment,
  keys: readonly string[],
) {
  for (const key of keys) {
    const value = nonEmptyString(environment[key]);
    if (value) {
      return value;
    }
  }
  return null;
}

function isAgentMessage(value: unknown): value is AnnouncementDemoAgentMessage {
  if (typeof value !== "object" || value === null) {
    return false;
  }

  const candidate = value as Record<string, unknown>;
  return (
    (candidate.role === "assistant" || candidate.role === "user") &&
    typeof candidate.content === "string" &&
    candidate.content.trim().length > 0
  );
}

export function resolveAnnouncementDemoAgentRequest(
  value: unknown,
  environment: AgentEnvironment = process.env,
): AnnouncementDemoAgentRequest {
  if (typeof value !== "object" || value === null) {
    throw new Error("The agent request was not valid JSON.");
  }

  const candidate = value as Record<string, unknown>;
  const submittedKey = nonEmptyString(candidate.apiKey);
  const openAiEnvironmentKey = firstEnvironmentValue(environment, [
    "OPENAI_COMPAT_API_KEY",
    "OPENAI_API_KEY",
  ]);
  const anthropicEnvironmentKey = firstEnvironmentValue(environment, [
    "ANTHROPIC_API_KEY",
  ]);
  const provider =
    supportedProvider(candidate.provider) ??
    supportedProvider(
      firstEnvironmentValue(environment, [
        "BUZZ_AGENT_PROVIDER",
        "GOOSE_PROVIDER",
      ]),
    ) ??
    (openAiEnvironmentKey
      ? "openai"
      : anthropicEnvironmentKey
        ? "anthropic"
        : null);
  if (!provider) {
    throw new Error("Choose Anthropic or OpenAI as the agent provider.");
  }

  const apiKey =
    submittedKey ??
    (provider === "openai" ? openAiEnvironmentKey : anthropicEnvironmentKey);
  if (!apiKey) {
    throw new Error("Add an API key in the agent settings first.");
  }

  const model =
    nonEmptyString(candidate.model) ??
    firstEnvironmentValue(environment, [
      "BUZZ_AGENT_MODEL",
      "GOOSE_MODEL",
      ...(provider === "openai"
        ? ["OPENAI_COMPAT_MODEL", "OPENAI_MODEL"]
        : ["ANTHROPIC_MODEL"]),
    ]);
  if (!model) {
    throw new Error("Choose a model in the agent settings first.");
  }
  if (
    typeof candidate.systemPrompt !== "string" ||
    !Array.isArray(candidate.messages) ||
    !candidate.messages.every(isAgentMessage)
  ) {
    throw new Error("The agent conversation context was incomplete.");
  }

  return {
    provider,
    apiKey,
    model,
    systemPrompt: candidate.systemPrompt,
    messages: candidate.messages,
  };
}

function readRequestBody(request: NodeJS.ReadableStream): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let bytesRead = 0;

    request.on("data", (chunk: Buffer) => {
      bytesRead += chunk.length;
      if (bytesRead > MAX_REQUEST_BYTES) {
        reject(new Error("The agent conversation was too large to send."));
        return;
      }
      chunks.push(chunk);
    });
    request.on("end", () => {
      try {
        resolve(JSON.parse(Buffer.concat(chunks).toString("utf8")));
      } catch {
        reject(new Error("The agent request was not valid JSON."));
      }
    });
    request.on("error", reject);
  });
}

function normalizeAnthropicModel(model: string) {
  if (model === "goose-claude-4-6-sonnet") {
    return "claude-sonnet-4-6";
  }
  if (model === "goose-claude-4-6-opus") {
    return "claude-opus-4-6";
  }
  return model.replace(/^anthropic\//, "");
}

function extractProviderError(body: ProviderErrorBody, status: number) {
  const nestedMessage =
    typeof body.error === "object" ? body.error.message : undefined;
  const message = nestedMessage ?? body.message ?? body.error;
  if (typeof message === "string" && message.trim()) {
    return `Provider request failed (${status}): ${message.trim()}`;
  }
  return `Provider request failed with status ${status}.`;
}

function extractOpenAiText(body: unknown) {
  if (typeof body !== "object" || body === null) {
    return null;
  }

  const response = body as {
    output_text?: unknown;
    output?: Array<{
      content?: Array<{ type?: unknown; text?: unknown }>;
    }>;
  };
  if (typeof response.output_text === "string" && response.output_text.trim()) {
    return response.output_text.trim();
  }

  const text = (response.output ?? [])
    .flatMap((item) => item.content ?? [])
    .filter((part) => part.type === "output_text")
    .map((part) => (typeof part.text === "string" ? part.text : ""))
    .join("")
    .trim();
  return text || null;
}

export function announcementDemoOpenAiReasoningEffort(
  model: string,
): OpenAiReasoningEffort | null {
  const normalizedModel = model.trim().toLowerCase();
  if (/^gpt-5\.(?:4|5|6)(?:[.-]|$)/.test(normalizedModel)) {
    return "none";
  }
  if (/^gpt-5(?:[.-]|$)/.test(normalizedModel)) {
    return "minimal";
  }
  if (/^o[1-9](?:[.-]|$)/.test(normalizedModel)) {
    return "low";
  }
  return null;
}

function extractAnthropicText(body: unknown) {
  if (typeof body !== "object" || body === null) {
    return null;
  }

  const response = body as {
    content?: Array<{ type?: unknown; text?: unknown }>;
  };
  const text = (response.content ?? [])
    .filter((part) => part.type === "text")
    .map((part) => (typeof part.text === "string" ? part.text : ""))
    .join("")
    .trim();
  return text || null;
}

async function requestOpenAiResponse(input: AnnouncementDemoAgentRequest) {
  const reasoningEffort = announcementDemoOpenAiReasoningEffort(input.model);
  const response = await fetch("https://api.openai.com/v1/responses", {
    method: "POST",
    headers: {
      Authorization: `Bearer ${input.apiKey}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      model: input.model,
      instructions: input.systemPrompt,
      input: input.messages,
      max_output_tokens: 2_000,
      ...(reasoningEffort ? { reasoning: { effort: reasoningEffort } } : {}),
      store: false,
    }),
    signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
  });
  const body = (await response.json()) as unknown;
  if (!response.ok) {
    throw new Error(
      extractProviderError(body as ProviderErrorBody, response.status),
    );
  }

  const text = extractOpenAiText(body);
  if (!text) {
    const responseDetails = body as {
      status?: unknown;
      incomplete_details?: { reason?: unknown };
    };
    if (
      responseDetails.status === "incomplete" &&
      responseDetails.incomplete_details?.reason === "max_output_tokens"
    ) {
      throw new Error(
        "OpenAI ran out of output tokens before producing visible text.",
      );
    }
    throw new Error("OpenAI returned a response without any text.");
  }
  return text;
}

async function requestAnthropicResponse(input: AnnouncementDemoAgentRequest) {
  const response = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: {
      "anthropic-version": "2023-06-01",
      "Content-Type": "application/json",
      "x-api-key": input.apiKey,
    },
    body: JSON.stringify({
      model: normalizeAnthropicModel(input.model),
      max_tokens: 350,
      system: input.systemPrompt,
      messages: input.messages,
    }),
    signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
  });
  const body = (await response.json()) as unknown;
  if (!response.ok) {
    throw new Error(
      extractProviderError(body as ProviderErrorBody, response.status),
    );
  }

  const text = extractAnthropicText(body);
  if (!text) {
    throw new Error("Anthropic returned a response without any text.");
  }
  return text;
}

function friendlyError(error: unknown) {
  if (error instanceof Error && error.name === "TimeoutError") {
    return "The model took too long to respond. Please try again.";
  }
  if (error instanceof Error) {
    return error.message;
  }
  return "The model could not respond just now.";
}

/**
 * Local-only provider proxy for the announcement demo. Keeping the API request
 * in Vite means provider credentials are never built into the frontend bundle
 * or written to source, and no Docker-backed relay is required.
 */
export function announcementDemoAgentPlugin(): Plugin {
  let agentEnvironment: AgentEnvironment = process.env;

  return {
    name: "buzz-announcement-demo-agent",
    configResolved(config) {
      agentEnvironment = {
        ...loadEnv(config.mode, config.envDir, ""),
        ...process.env,
      };
    },
    configureServer(server) {
      server.middlewares.use(AGENT_RESPONSE_PATH, async (request, response) => {
        response.setHeader("Content-Type", "application/json");
        response.setHeader("Cache-Control", "no-store");

        if (request.method !== "POST") {
          response.statusCode = 405;
          response.end(JSON.stringify({ error: "Method not allowed." }));
          return;
        }

        try {
          const input = resolveAnnouncementDemoAgentRequest(
            await readRequestBody(request),
            agentEnvironment,
          );
          const text =
            input.provider === "openai"
              ? await requestOpenAiResponse(input)
              : await requestAnthropicResponse(input);
          response.statusCode = 200;
          response.end(JSON.stringify({ text }));
        } catch (error) {
          const message = friendlyError(error);
          server.config.logger.error(`[announcement-demo-agent] ${message}`);
          response.statusCode = 502;
          response.end(JSON.stringify({ error: message }));
        }
      });
      server.middlewares.use(
        MOCK_RELAY_QUERY_PATH,
        async (request, response) => {
          response.setHeader("Content-Type", "application/json");
          response.setHeader("Cache-Control", "no-store");

          if (request.method !== "POST") {
            response.statusCode = 405;
            response.end(JSON.stringify({ error: "Method not allowed." }));
            return;
          }

          try {
            await readRequestBody(request);
            // Announcement state is served by the in-browser bridge. This
            // endpoint is only a safety net for a native or direct-fetch code
            // path that bypasses the bridge; it must never reach a real relay.
            response.statusCode = 200;
            response.end(JSON.stringify([]));
          } catch (error) {
            response.statusCode = 400;
            response.end(JSON.stringify({ error: friendlyError(error) }));
          }
        },
      );
    },
  };
}
