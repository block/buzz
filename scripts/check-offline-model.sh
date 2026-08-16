#!/usr/bin/env bash
set -euo pipefail

endpoint="http://127.0.0.1:1234"
model="google/gemma-4-26b-a4b"
instance="gemma4-26b-official"
report=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --endpoint) endpoint=${2:?}; shift 2 ;;
    --model) model=${2:?}; shift 2 ;;
    --instance) instance=${2:?}; shift 2 ;;
    --report) report=${2:?}; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ ! "$endpoint" =~ ^http://127\.0\.0\.1:[1-9][0-9]{0,4}$ ]]; then
  echo "offline model check requires a literal IPv4 loopback HTTP endpoint" >&2
  exit 2
fi

work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' EXIT
curl_lmstudio() {
  if [[ -n "${LM_STUDIO_API_TOKEN:-}" ]]; then
    command curl -H "Authorization: Bearer ${LM_STUDIO_API_TOKEN}" "$@"
  else
    command curl "$@"
  fi
}

catalog="$work_dir/catalog.json"
curl_lmstudio --silent --show-error --fail --max-time 10 \
  "$endpoint/api/v1/models" >"$catalog"

if ! jq -e --arg model "$model" --arg instance "$instance" '
  .models[]
  | select(.type == "llm" and .key == $model)
  | select(.capabilities.vision == true and .capabilities.trained_for_tool_use == true)
  | .loaded_instances[]
  | select(.id == $instance)
  | select(.config.context_length == 65536 and .config.parallel == 1)
' "$catalog" >/dev/null; then
  echo "qualified Gemma runtime is not admitted at 64K with parallelism one" >&2
  exit 3
fi

request="$work_dir/request.json"
response="$work_dir/response.json"
jq -n --arg instance "$instance" '{
  model: $instance,
  input: "Reply exactly GEMMA64 READY",
  system_prompt: "Follow the user instruction exactly.",
  stream: false,
  reasoning: "off",
  max_output_tokens: 256,
  context_length: 65536,
  store: false
}' >"$request"

started=$(date -u +%Y-%m-%dT%H:%M:%SZ)
start_seconds=$SECONDS
curl_lmstudio --silent --show-error --fail --max-time 900 \
  -H "Content-Type: application/json" \
  --data-binary "@$request" "$endpoint/api/v1/chat" >"$response"
elapsed=$((SECONDS - start_seconds))

if ! jq -e --arg instance "$instance" '
  .model_instance_id == $instance
  and .stats.reasoning_output_tokens == 0
  and ([.output[] | select(.type == "message") | .content] | join("\n") == "GEMMA64 READY")
' "$response" >/dev/null; then
  echo "qualified runtime exact-text or reasoning-off canary failed" >&2
  exit 4
fi

result=$(jq -n \
  --arg timestamp "$started" \
  --arg model "$model" \
  --arg instance "$instance" \
  --argjson elapsed "$elapsed" \
  --argjson input "$(jq '.stats.input_tokens' "$response")" \
  --argjson output "$(jq '.stats.total_output_tokens' "$response")" \
  '{
    timestamp: $timestamp,
    endpoint: "loopback",
    modelId: $model,
    instanceId: $instance,
    contextLength: 65536,
    maxOutputTokens: 8192,
    reasoning: "off",
    generationCapacity: 1,
    elapsedSeconds: $elapsed,
    inputTokens: $input,
    outputTokens: $output,
    result: "pass"
  }')

if [[ -n "$report" ]]; then
  mkdir -p "$(dirname "$report")"
  printf '%s\n' "$result" >"$report"
fi
printf '%s\n' "$result"
