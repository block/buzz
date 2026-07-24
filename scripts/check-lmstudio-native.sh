#!/usr/bin/env bash
set -euo pipefail

base_url="${LM_STUDIO_BASE_URL:-http://127.0.0.1:1234}"
configured_model="${LM_STUDIO_MODEL:-}"
smoke=false
reasoning="off"
token_from_stdin=false
keychain_service=""
catalog_limit_bytes=1048576
chat_limit_bytes=2097152
connect_timeout_seconds=2
request_timeout_seconds=30
runtime_tmp=$(mktemp -d)
token="${LM_STUDIO_API_TOKEN:-}"

cleanup() {
  rm -rf "${runtime_tmp}"
}
trap cleanup EXIT

usage() {
  cat <<'EOF'
Usage: scripts/check-lmstudio-native.sh [options]

Read the native LM Studio model catalog and optionally run a bounded chat
smoke test. This command never loads, unloads, or downloads a model and never
changes LM Studio configuration.

Options:
  --base-url URL              Literal loopback API root (default:
                              http://127.0.0.1:1234)
  --model ID                  Required loaded model, or auto-select when exactly
                              one LLM model is loaded
  --smoke                     Perform a bounded native /api/v1/chat check
  --reasoning off|on          Native reasoning option for --smoke (default: off)
  --token-stdin               Read one bearer token line from standard input
  --keychain-service SERVICE  Read key "lm-studio-api-token" from the Buzz
                              SecretStore JSON blob for SERVICE
  -h, --help                  Show this help

LM_STUDIO_API_TOKEN is also accepted as a bearer-token input. Tokens are never
printed. A token is sent only after a tokenless catalog request returns 401/403.
EOF
}

fail_config() {
  echo "[lmstudio-native] configuration denied" >&2
  exit 2
}

valid_safe_identifier() {
  local value="$1"
  local max_bytes="$2"
  printf '%s' "${value}" |
    /usr/bin/jq -Rse --argjson max_bytes "${max_bytes}" '
      (utf8bytelength > 0)
      and (utf8bytelength <= $max_bytes)
      and (test("[\u0000-\u001f\u007f]") | not)
    ' >/dev/null 2>&1
}

validate_base_url() {
  local raw="$1"
  local authority port
  case "${raw}" in
    http://127.0.0.1:* | http://\[\:\:1\]:*) ;;
    *) return 1 ;;
  esac
  if [[ "${raw}" == */ ]]; then
    raw="${raw%/}"
  fi
  authority="${raw#http://}"
  if [[ "${authority}" == 127.0.0.1:* ]]; then
    port="${authority#127.0.0.1:}"
  elif [[ "${authority}" == \[\:\:1\]:* ]]; then
    port="${authority#\[\:\:1\]:}"
  else
    return 1
  fi
  [[ "${port}" =~ ^[1-9][0-9]{0,4}$ ]] || return 1
  ((10#${port} >= 1 && 10#${port} <= 65535)) || return 1
  # Rust's URL parser normalizes HTTP's explicit default port to None; match
  # that fail-closed policy instead of accepting a route Rust would deny.
  ((10#${port} != 80)) || return 1
  normalized_base_url="${raw}"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base-url)
      [[ $# -ge 2 ]] || fail_config
      base_url="$2"
      shift 2
      ;;
    --model)
      [[ $# -ge 2 ]] || fail_config
      configured_model="$2"
      shift 2
      ;;
    --smoke)
      smoke=true
      shift
      ;;
    --reasoning)
      [[ $# -ge 2 ]] || fail_config
      reasoning="$2"
      shift 2
      ;;
    --token-stdin)
      token_from_stdin=true
      shift
      ;;
    --keychain-service)
      [[ $# -ge 2 ]] || fail_config
      keychain_service="$2"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      fail_config
      ;;
  esac
done

validate_base_url "${base_url}" || fail_config
valid_safe_identifier "${configured_model:-auto}" 256 || fail_config
[[ "${reasoning}" == "off" || "${reasoning}" == "on" ]] || fail_config
if [[ "${token_from_stdin}" == "true" && -n "${keychain_service}" ]]; then
  fail_config
fi
if [[ "${token_from_stdin}" == "true" && -n "${token}" ]]; then
  fail_config
fi

if [[ "${token_from_stdin}" == "true" ]]; then
  IFS= read -r token || {
    echo "[lmstudio-native] token input unavailable" >&2
    exit 2
  }
fi

if [[ -n "${keychain_service}" ]]; then
  valid_safe_identifier "${keychain_service}" 128 || fail_config
  [[ -z "${token}" ]] || fail_config
  keychain_blob=$(
    /usr/bin/security find-generic-password \
      -s "${keychain_service}" \
      -a secrets \
      -w 2>/dev/null
  ) || {
    echo "[lmstudio-native] keychain token unavailable" >&2
    exit 2
  }
  token=$(
    printf '%s' "${keychain_blob}" |
      /usr/bin/jq -er '
        .["lm-studio-api-token"]
        | select(type == "string" and (utf8bytelength > 0) and (utf8bytelength <= 4096))
      ' 2>/dev/null
  ) || {
    echo "[lmstudio-native] keychain token unavailable" >&2
    exit 2
  }
  keychain_blob=""
fi

if [[ -n "${token}" ]]; then
  valid_safe_identifier "${token}" 4096 || fail_config
fi

auth_header_file=""
if [[ -n "${token}" ]]; then
  auth_header_file="${runtime_tmp}/authorization-header"
  umask 077
  printf 'Authorization: Bearer %s\n' "${token}" >"${auth_header_file}"
fi

request() {
  local method="$1"
  local path="$2"
  local output_file="$3"
  local max_bytes="$4"
  local authenticated="$5"
  local input_file="${6:-}"
  local curl_status http_status
  local -a curl_args=(
    --silent
    --show-error
    --noproxy "*"
    --proxy ""
    --max-redirs 0
    --proto "=http"
    --connect-timeout "${connect_timeout_seconds}"
    --max-time "${request_timeout_seconds}"
    --max-filesize "${max_bytes}"
    --request "${method}"
    --header "Accept: application/json"
    --output "${output_file}"
    --write-out "%{http_code}"
  )
  if [[ "${authenticated}" == "true" ]]; then
    [[ -n "${auth_header_file}" ]] || return 67
    curl_args+=(--header "@${auth_header_file}")
  fi
  if [[ -n "${input_file}" ]]; then
    curl_args+=(
      --header "Content-Type: application/json"
      --data-binary "@${input_file}"
    )
  fi
  if http_status=$(
    env \
      -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY \
      -u http_proxy -u https_proxy -u all_proxy \
      /usr/bin/curl "${curl_args[@]}" "${normalized_base_url}${path}" 2>/dev/null
  ); then
    printf '%s\n' "${http_status}"
    return 0
  else
    curl_status=$?
    return "${curl_status}"
  fi
}

catalog_file="${runtime_tmp}/models.json"
catalog_status=""
if catalog_status=$(request GET /api/v1/models "${catalog_file}" "${catalog_limit_bytes}" false); then
  :
else
  request_status=$?
  if [[ ${request_status} -eq 63 ]]; then
    echo "[lmstudio-native] state: model catalog exceeded ${catalog_limit_bytes} bytes" >&2
  else
    echo "[lmstudio-native] state: unreachable" >&2
  fi
  exit 3
fi

authentication="not enforced"
case "${catalog_status}" in
  200) ;;
  401 | 403)
    authentication="enforced"
    if [[ -z "${token}" ]]; then
      echo "[lmstudio-native] state: authentication required" >&2
      exit 4
    fi
    if catalog_status=$(request GET /api/v1/models "${catalog_file}" "${catalog_limit_bytes}" true); then
      :
    else
      request_status=$?
      if [[ ${request_status} -eq 63 ]]; then
        echo "[lmstudio-native] state: model catalog exceeded ${catalog_limit_bytes} bytes" >&2
      else
        echo "[lmstudio-native] state: unreachable" >&2
      fi
      exit 3
    fi
    if [[ "${catalog_status}" == "401" || "${catalog_status}" == "403" ]]; then
      echo "[lmstudio-native] state: authentication required" >&2
      exit 4
    fi
    ;;
  *)
    echo "[lmstudio-native] state: API error (HTTP ${catalog_status})" >&2
    exit 3
    ;;
esac
if [[ "${catalog_status}" != "200" ]]; then
  echo "[lmstudio-native] state: API error (HTTP ${catalog_status})" >&2
  exit 3
fi

catalog_schema='
  def safe_id:
    type == "string"
    and (utf8bytelength > 0)
    and (utf8bytelength <= 256)
    and test("^[^\u0000-\u001f\u007f]+$");
  type == "object"
  and (keys == ["models"])
  and (.models | type == "array" and length <= 256)
  and all(.models[];
    type == "object"
    and (.key | safe_id)
    and (.type | type == "string")
    and (.loaded_instances | type == "array" and length <= 32)
    and all(.loaded_instances[];
      type == "object" and (.id | safe_id)
    )
    and (
      .type != "llm"
      or (
        (.max_context_length == null or (
          (.max_context_length | type == "number")
          and (.max_context_length | floor == .)
          and (.max_context_length > 0)
          and (.max_context_length <= 16777216)
        ))
        and (.display_name == null or (
          (.display_name | type == "string")
          and (.display_name | utf8bytelength <= 512)
        ))
        and (.description == null or (
          (.description | type == "string")
          and (.description | utf8bytelength <= 4096)
        ))
        and (.capabilities == null or (.capabilities | type == "object"))
      )
    )
  )
'
if ! /usr/bin/jq -e "${catalog_schema}" "${catalog_file}" >/dev/null 2>&1; then
  echo "[lmstudio-native] state: malformed model catalog" >&2
  exit 5
fi

loaded_count=$(
  /usr/bin/jq '[.models[] | select(.type == "llm" and (.loaded_instances | length > 0))] | length' \
    "${catalog_file}"
)
if ((loaded_count == 0)); then
  echo "[lmstudio-native] authentication: ${authentication}"
  echo "[lmstudio-native] bind exposure: unknown"
  echo "[lmstudio-native] state: no loaded LLM model" >&2
  exit 6
fi

if [[ -z "${configured_model}" ]]; then
  if ((loaded_count != 1)); then
    echo "[lmstudio-native] state: multiple loaded LLM models; configure --model" >&2
    exit 2
  fi
  configured_model=$(
    /usr/bin/jq -r \
      '.models[] | select(.type == "llm" and (.loaded_instances | length > 0)) | .key' \
      "${catalog_file}"
  )
fi
valid_safe_identifier "${configured_model}" 256 || fail_config

model_loaded=$(
  /usr/bin/jq -r --arg model "${configured_model}" '
    any(.models[];
      .type == "llm"
      and .key == $model
      and (.loaded_instances | length > 0)
    )
  ' "${catalog_file}"
)
if [[ "${model_loaded}" != "true" ]]; then
  echo "[lmstudio-native] authentication: ${authentication}"
  echo "[lmstudio-native] bind exposure: unknown"
  echo "[lmstudio-native] state: configured model unavailable" >&2
  exit 7
fi

echo "[lmstudio-native] authentication: ${authentication}"
echo "[lmstudio-native] bind exposure: unknown"
echo "[lmstudio-native] loaded LLM models: ${loaded_count}"
echo "[lmstudio-native] loaded model: ${configured_model}"
if [[ "${authentication}" == "not enforced" ]]; then
  echo "[lmstudio-native] security warning: API authentication is not enforced"
fi

if [[ "${smoke}" != "true" ]]; then
  echo "[lmstudio-native] state: ready"
  exit 0
fi

reasoning_supported=$(
  /usr/bin/jq -r --arg model "${configured_model}" --arg reasoning "${reasoning}" '
    any(.models[];
      .type == "llm"
      and .key == $model
      and ((.capabilities.reasoning.allowed_options // []) | index($reasoning) != null)
    )
  ' "${catalog_file}"
)
if [[ "${reasoning_supported}" != "true" ]]; then
  echo "[lmstudio-native] state: requested reasoning option unavailable" >&2
  exit 8
fi

context_length=$(
  /usr/bin/jq -r --arg model "${configured_model}" '
    [
      .models[]
      | select(.type == "llm" and .key == $model)
      | (.max_context_length // 32768)
    ][0]
    | if . > 32768 then 32768 else . end
  ' "${catalog_file}"
)
chat_request="${runtime_tmp}/chat-request.json"
/usr/bin/jq -cn \
  --arg model "${configured_model}" \
  --arg reasoning "${reasoning}" \
  --argjson context_length "${context_length}" '
    {
      model: $model,
      input: "Answer exactly: OK",
      system_prompt: "No tools. Answer immediately without elaboration.",
      stream: false,
      reasoning: $reasoning,
      max_output_tokens: 256,
      context_length: $context_length,
      store: true
    }
  ' >"${chat_request}"

chat_file="${runtime_tmp}/chat.json"
if chat_status=$(
  request POST /api/v1/chat "${chat_file}" "${chat_limit_bytes}" \
    "$([[ "${authentication}" == "enforced" ]] && echo true || echo false)" \
    "${chat_request}"
); then
  :
else
  request_status=$?
  if [[ ${request_status} -eq 63 ]]; then
    echo "[lmstudio-native] state: chat response exceeded ${chat_limit_bytes} bytes" >&2
  else
    echo "[lmstudio-native] state: chat unavailable" >&2
  fi
  exit 9
fi
if [[ "${chat_status}" == "401" || "${chat_status}" == "403" ]]; then
  echo "[lmstudio-native] state: authentication required" >&2
  exit 4
fi
if [[ "${chat_status}" != "200" ]]; then
  echo "[lmstudio-native] state: chat API error (HTTP ${chat_status})" >&2
  exit 9
fi

chat_schema='
  def safe_id:
    type == "string"
    and (utf8bytelength > 0)
    and (utf8bytelength <= 256)
    and test("^[^\u0000-\u001f\u007f]+$");
  def bounded_text:
    type == "string" and utf8bytelength <= 1048576;
  type == "object"
  and (keys == ["model_instance_id", "output", "response_id", "stats"])
  and (.model_instance_id | safe_id)
  and (.response_id |
    type == "string"
    and utf8bytelength <= 261
    and test("^resp_[A-Za-z0-9_-]+$")
  )
  and (.output | type == "array" and length > 0 and length <= 1024)
  and (.output[-1].type == "message")
  and all(.output[];
    if .type == "message" or .type == "reasoning" then
      (keys == ["content", "type"]) and (.content | bounded_text)
    elif .type == "tool_call" then
      (keys == ["arguments", "output", "provider_info", "tool", "type"])
      and (.tool | safe_id)
      and (.arguments | type == "object")
      and ((.arguments | tojson | utf8bytelength) <= 65536)
      and (.output | bounded_text)
      and (
        (.provider_info | type == "object")
        and (
          (
            .provider_info.type == "ephemeral_mcp"
            and (.provider_info | keys == ["server_label", "type"])
            and (.provider_info.server_label | safe_id)
          )
          or (
            .provider_info.type == "plugin"
            and (.provider_info | keys == ["plugin_id", "type"])
            and (.provider_info.plugin_id | safe_id)
          )
        )
      )
    else
      false
    end
  )
  and (.stats | type == "object")
  and all([
    .stats.input_tokens,
    .stats.total_output_tokens,
    .stats.reasoning_output_tokens
  ][];
    type == "number" and floor == . and . >= 0 and . <= 9007199254740991
  )
'
if ! /usr/bin/jq -e "${chat_schema}" "${chat_file}" >/dev/null 2>&1; then
  echo "[lmstudio-native] state: malformed chat response" >&2
  exit 10
fi

model_instance_id=$(/usr/bin/jq -r '.model_instance_id' "${chat_file}")
instance_authorized=$(
  /usr/bin/jq -r --arg model "${configured_model}" --arg instance "${model_instance_id}" '
    any(.models[];
      .type == "llm"
      and .key == $model
      and (
        any(.loaded_instances[]; .id == $instance)
        or (
          ($instance | startswith($model + ":"))
          and (
            $instance[($model | length) + 1:]
            | test("^[1-9][0-9]*$")
          )
        )
      )
    )
  ' "${catalog_file}"
)
if [[ "${instance_authorized}" != "true" ]]; then
  echo "[lmstudio-native] state: chat used an unexpected model instance" >&2
  exit 10
fi

output_types=$(
  /usr/bin/jq -r '[.output[].type] | join(",")' "${chat_file}"
)
tool_call_count=$(
  /usr/bin/jq '[.output[] | select(.type == "tool_call")] | length' "${chat_file}"
)
stats=$(
  /usr/bin/jq -r '
    "input=\(.stats.input_tokens) output=\(.stats.total_output_tokens) reasoning=\(.stats.reasoning_output_tokens)"
  ' "${chat_file}"
)

echo "[lmstudio-native] reasoning: ${reasoning}"
echo "[lmstudio-native] model instance: ${model_instance_id}"
echo "[lmstudio-native] output item types: ${output_types}"
echo "[lmstudio-native] native tool_call items: ${tool_call_count}"
echo "[lmstudio-native] stats: ${stats}"
echo "[lmstudio-native] response_id: valid"
echo "[lmstudio-native] state: ready"
