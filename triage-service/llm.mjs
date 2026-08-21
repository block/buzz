const DEFAULT_MODEL = "gpt-4o-mini";

/**
 * The LLM is the primary classifier. It is only skipped when there is no key
 * to call with, or when the operator explicitly asks for the heuristic with
 * `TRIAGE_LLM=0`.
 */
export function llmEnabled() {
  if (process.env.TRIAGE_LLM === "0") return false;
  return Boolean(process.env.OPENAI_API_KEY);
}

export function llmModel() {
  return process.env.TRIAGE_MODEL ?? DEFAULT_MODEL;
}

export function describeMode() {
  return llmEnabled() ? `LLM (${llmModel()})` : "heuristic";
}

export async function callLlmJson(prompt) {
  const apiKey = process.env.OPENAI_API_KEY;
  if (!apiKey) throw new Error("OPENAI_API_KEY is not set");

  const response = await fetch("https://api.openai.com/v1/chat/completions", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${apiKey}`,
    },
    body: JSON.stringify({
      model: llmModel(),
      response_format: { type: "json_object" },
      messages: [{ role: "user", content: prompt }],
    }),
  });

  if (!response.ok) {
    throw new Error(`OpenAI request failed: ${response.status}`);
  }

  const payload = await response.json();
  return JSON.parse(payload.choices?.[0]?.message?.content ?? "{}");
}
