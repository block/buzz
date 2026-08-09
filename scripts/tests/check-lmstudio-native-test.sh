#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
checker="${repo_root}/scripts/check-lmstudio-native.sh"
fake_server="${repo_root}/scripts/tests/lmstudio-native-fake-server.mjs"
justfile="${repo_root}/Justfile"
test_tmp=$(mktemp -d)
server_pids=()

cleanup() {
  local pid
  for pid in "${server_pids[@]-}"; do
    [[ -n "${pid}" ]] || continue
    kill "${pid}" 2>/dev/null || true
    wait "${pid}" 2>/dev/null || true
  done
  rm -rf "${test_tmp}"
}
trap cleanup EXIT

fail() {
  echo "check-lmstudio-native test failed: $*" >&2
  exit 1
}

assert_contains() {
  local file="$1"
  local expected="$2"
  grep -Fq "${expected}" "${file}" ||
    fail "expected $(basename "${file}") to contain: ${expected}"
}

assert_not_contains() {
  local file="$1"
  local unexpected="$2"
  if grep -Fq "${unexpected}" "${file}"; then
    fail "unexpected sensitive text in $(basename "${file}"): ${unexpected}"
  fi
}

start_server() {
  local mode="$1"
  local name="$2"
  local token="${3:-test-token}"
  local chat_variant="${4:-message}"
  server_port_file="${test_tmp}/${name}.port"
  server_request_log="${test_tmp}/${name}.requests"
  server_payload_log="${test_tmp}/${name}.payloads"
  : >"${server_request_log}"
  : >"${server_payload_log}"

  FAKE_LMSTUDIO_MODE="${mode}" \
    FAKE_LMSTUDIO_TOKEN="${token}" \
    FAKE_LMSTUDIO_CHAT_VARIANT="${chat_variant}" \
    node "${fake_server}" \
      "${server_port_file}" \
      "${server_request_log}" \
      "${server_payload_log}" \
      >"${test_tmp}/${name}.stdout" 2>"${test_tmp}/${name}.stderr" &
  server_pid=$!
  server_pids+=("${server_pid}")

  local ticks=0
  while [[ ! -s "${server_port_file}" ]] && ((ticks < 100)); do
    if ! kill -0 "${server_pid}" 2>/dev/null; then
      fail "fake server ${name} exited before listening"
    fi
    sleep 0.05
    ticks=$((ticks + 1))
  done
  [[ -s "${server_port_file}" ]] || fail "fake server ${name} did not listen"
  server_port=$(<"${server_port_file}")
  server_base_url="http://127.0.0.1:${server_port}"
}

run_failure() {
  local output_file="$1"
  shift
  if "$@" >"${output_file}" 2>&1; then
    fail "expected command to fail: $*"
  fi
}

[[ -x "${checker}" ]] || fail "checker is missing or not executable"
[[ -f "${fake_server}" ]] || fail "fake server fixture is missing"

# Endpoint policy must reject before curl and before an ambient proxy can see a request.
start_server valid denied-proxy
proxy_request_log="${server_request_log}"
denied_output="${test_tmp}/denied.out"
run_failure "${denied_output}" \
  env HTTP_PROXY="${server_base_url}" HTTPS_PROXY="${server_base_url}" ALL_PROXY="${server_base_url}" \
  "${checker}" --base-url "http://10.0.0.10:1234"
assert_contains "${denied_output}" "configuration denied"
[[ ! -s "${proxy_request_log}" ]] || fail "denied endpoint emitted a proxy request"

for denied_url in \
  "http://localhost:1234" \
  "http://127.0.0.1" \
  "http://127.0.0.1:080" \
  "http://127.0.0.1:80" \
  "http://127.0.0.1:65536" \
  "https://127.0.0.1:1234" \
  "http://[::ffff:127.0.0.1]:1234" \
  "http://127.0.0.1:1234/path" \
  "http://127.0.0.1:1234?query=1"; do
  run_failure "${test_tmp}/denied-url.out" \
    "${checker}" --base-url "${denied_url}"
  assert_contains "${test_tmp}/denied-url.out" "configuration denied"
done

# Identifier and token controls must be byte-bounded and reject embedded
# newlines before curl can turn them into request/header material.
start_server valid denied-inputs
denied_inputs_log="${server_request_log}"
run_failure "${test_tmp}/denied-model.out" \
  "${checker}" \
    --base-url "${server_base_url}" \
    --model $'qwen/test-model\nInjected: value'
assert_contains "${test_tmp}/denied-model.out" "configuration denied"
run_failure "${test_tmp}/denied-token.out" \
  env LM_STUDIO_API_TOKEN=$'secret\nX-Injected: value' \
  "${checker}" \
    --base-url "${server_base_url}" \
    --model qwen/test-model
assert_contains "${test_tmp}/denied-token.out" "configuration denied"
oversized_multibyte_model=$(
  printf '\303\251%.0s' {1..129}
)
run_failure "${test_tmp}/denied-multibyte-model.out" \
  "${checker}" \
    --base-url "${server_base_url}" \
    --model "${oversized_multibyte_model}"
assert_contains "${test_tmp}/denied-multibyte-model.out" "configuration denied"
[[ ! -s "${denied_inputs_log}" ]] ||
  fail "denied model or token input emitted a request"

# A valid literal-loopback request must bypass all ambient proxy variables.
start_server valid proxy
valid_proxy_url="${server_base_url}"
valid_proxy_log="${server_request_log}"
start_server valid target
target_log="${server_request_log}"
valid_output="${test_tmp}/valid.out"
HTTP_PROXY="${valid_proxy_url}" \
  HTTPS_PROXY="${valid_proxy_url}" \
  ALL_PROXY="${valid_proxy_url}" \
  http_proxy="${valid_proxy_url}" \
  https_proxy="${valid_proxy_url}" \
  all_proxy="${valid_proxy_url}" \
  "${checker}" \
    --base-url "${server_base_url}" \
    --model qwen/test-model >"${valid_output}" 2>&1
assert_contains "${valid_output}" "state: ready"
assert_contains "${valid_output}" "loaded model: qwen/test-model"
[[ -s "${target_log}" ]] || fail "valid target saw no request"
[[ ! -s "${valid_proxy_log}" ]] || fail "valid request used an ambient proxy"

# Unreachable, auth-required, no-loaded, mismatch, malformed, and oversized
# responses must remain distinct and must not reflect response bodies.
unused_port=9
run_failure "${test_tmp}/unreachable.out" \
  "${checker}" --base-url "http://127.0.0.1:${unused_port}"
assert_contains "${test_tmp}/unreachable.out" "state: unreachable"

start_server auth auth-required very-private-token
auth_output="${test_tmp}/auth-required.out"
run_failure "${auth_output}" \
  "${checker}" --base-url "${server_base_url}"
assert_contains "${auth_output}" "state: authentication required"
assert_not_contains "${auth_output}" "PRIVATE_AUTH_BODY"
assert_not_contains "${auth_output}" "very-private-token"

authenticated_output="${test_tmp}/authenticated.out"
printf '%s\n' "very-private-token" |
  "${checker}" \
    --base-url "${server_base_url}" \
    --model qwen/test-model \
    --token-stdin >"${authenticated_output}" 2>&1
assert_contains "${authenticated_output}" "authentication: enforced"
assert_contains "${authenticated_output}" "state: ready"
assert_not_contains "${authenticated_output}" "very-private-token"
assert_not_contains "${authenticated_output}" "PRIVATE_AUTH_BODY"

start_server no-loaded no-loaded
run_failure "${test_tmp}/no-loaded.out" \
  "${checker}" --base-url "${server_base_url}"
assert_contains "${test_tmp}/no-loaded.out" "state: no loaded LLM model"

start_server valid mismatch
run_failure "${test_tmp}/mismatch.out" \
  "${checker}" --base-url "${server_base_url}" --model missing/model
assert_contains "${test_tmp}/mismatch.out" "state: configured model unavailable"

start_server malformed malformed
run_failure "${test_tmp}/malformed.out" \
  "${checker}" --base-url "${server_base_url}"
assert_contains "${test_tmp}/malformed.out" "state: malformed model catalog"
assert_not_contains "${test_tmp}/malformed.out" "PRIVATE_MALFORMED"

start_server oversize oversize
run_failure "${test_tmp}/oversize.out" \
  "${checker}" --base-url "${server_base_url}"
assert_contains "${test_tmp}/oversize.out" "state: model catalog exceeded"

# Smoke mode reports only validated shape metadata. It must not print request,
# response, tool argument, tool output, or bearer content.
start_server chat pseudo-smoke test-token pseudo-tool
pseudo_output="${test_tmp}/pseudo.out"
"${checker}" \
  --base-url "${server_base_url}" \
  --model qwen/test-model \
  --smoke \
  --reasoning off >"${pseudo_output}" 2>&1
assert_contains "${pseudo_output}" "reasoning: off"
assert_contains "${pseudo_output}" "output item types: reasoning,message"
assert_contains "${pseudo_output}" "native tool_call items: 0"
assert_contains "${pseudo_output}" "stats: input=7 output=5 reasoning=2"
assert_contains "${pseudo_output}" "response_id: valid"
for sensitive in \
  PRIVATE_FAKE_RESPONSE_CONTENT \
  PRIVATE_FAKE_PROMPT \
  PRIVATE_FAKE_TOOL_OUTPUT \
  tool_call\>\{; do
  assert_not_contains "${pseudo_output}" "${sensitive}"
done
assert_contains "${server_payload_log}" '"reasoning":"off"'
assert_not_contains "${server_payload_log}" '"tools"'
assert_not_contains "${server_payload_log}" '"tool_choice"'

start_server chat tool-smoke test-token tool-call
tool_server_base_url="${server_base_url}"
tool_output="${test_tmp}/tool.out"
"${checker}" \
  --base-url "${server_base_url}" \
  --model qwen/test-model \
  --smoke \
  --reasoning on >"${tool_output}" 2>&1
assert_contains "${tool_output}" "reasoning: on"
assert_contains "${tool_output}" "output item types: tool_call,message"
assert_contains "${tool_output}" "native tool_call items: 1"
assert_not_contains "${tool_output}" "PRIVATE_FAKE_TOOL_OUTPUT"
assert_contains "${server_payload_log}" '"reasoning":"on"'
assert_contains "${server_payload_log}" '"max_output_tokens":256'

# LM Studio's live native response appends a bounded parallel-slot suffix to
# model_instance_id even though the catalog reports the unsuffixed instance.
start_server chat parallel-instance test-token parallel-instance
parallel_output="${test_tmp}/parallel.out"
"${checker}" \
  --base-url "${server_base_url}" \
  --model qwen/test-model \
  --smoke \
  --reasoning off >"${parallel_output}" 2>&1
assert_contains "${parallel_output}" "model instance: qwen/test-model:2"

# The Just entrypoint must remain non-mutating and expose the bounded smoke
# switch without reconstructing HTTP behavior itself.
just_output="${test_tmp}/just.out"
LM_STUDIO_BASE_URL="${tool_server_base_url}" \
  LM_STUDIO_MODEL=qwen/test-model \
  BUZZ_LMSTUDIO_SMOKE=1 \
  BUZZ_LMSTUDIO_REASONING=on \
  just --justfile "${justfile}" check-lmstudio-native >"${just_output}" 2>&1
assert_contains "${just_output}" "output item types: tool_call,message"

echo "LM Studio native checker contract passed"
