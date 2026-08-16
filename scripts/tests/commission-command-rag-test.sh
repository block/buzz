#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
commissioner="${repo_root}/scripts/commission-command-rag.sh"
test_tmp="$(mktemp -d)"
trap 'rm -rf "${test_tmp}"' EXIT

fail() {
  printf 'commission-command-rag test failed: %s\n' "$*" >&2
  exit 1
}

config="${test_tmp}/trusted-lan-sources.json"
cat >"${config}" <<'JSON'
{
  "schema_version": 1,
  "mode": "OFFICIAL_TRUSTED_LAN",
  "memory_url": "http://192.168.1.26:8006/mcp",
  "rag_url": "http://192.168.1.107:8005/mcp/",
  "automatic_cloud_fallback_acknowledged": true,
  "litellm": {"enabled": true, "endpoint": "http://192.168.1.26:4000/v1/chat/completions", "model": "chatgpt-5.4", "keychain_key": "command.cloud.litellm"},
  "openai": {"enabled": true, "endpoint": "https://api.openai.com/v1/responses", "model": "gpt-5.4", "keychain_key": "command.cloud.openai"}
}
JSON
chmod 600 "${config}"

mock_curl="${test_tmp}/curl"
cat >"${mock_curl}" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
url="${*: -1}"
if [[ "${url}" == */health ]]; then
  printf '%s\n' '{"status":"ok","points":123502}'
else
  printf '%s\n' '{"diagnostics":{"snapshot_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"results":[{"point_id":"point-1","doc_name":"ADFP_5.0.1.pdf","collection":"ADF Doctrine","page_no":15,"section_path":["Mission analysis"]}]}'
fi
SH
chmod +x "${mock_curl}"

"${commissioner}" \
  --config "${config}" \
  --endpoint http://127.0.0.1:8005/mcp/ \
  --snapshot-id aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --collection "ADF Doctrine" \
  --query "Joint Military Appreciation Process mission analysis" \
  --curl "${mock_curl}" >"${test_tmp}/success.out"

[[ "$(jq -r .rag_url "${config}")" == "http://127.0.0.1:8005/mcp/" ]] ||
  fail "loopback RAG endpoint was not installed"
[[ "$(stat -f %Lp "${config}")" == "600" ]] || fail "config permissions changed"
grep -Fq 'commissioned snapshot aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' \
  "${test_tmp}/success.out" || fail "success evidence was not printed"

PATH="${test_tmp}:${PATH}" "${commissioner}" \
  --config "${config}" \
  --endpoint http://127.0.0.1:8005/mcp/ \
  --snapshot-id aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --collection "ADF Doctrine" \
  --query "Joint Military Appreciation Process mission analysis" >/dev/null

before="$(shasum -a 256 "${config}" | awk '{print $1}')"
if "${commissioner}" \
  --config "${config}" \
  --endpoint http://192.168.1.107:8005/mcp/ \
  --snapshot-id aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
  --collection "ADF Doctrine" \
  --query "Joint Military Appreciation Process mission analysis" \
  --curl "${mock_curl}" >/dev/null 2>&1; then
  fail "remote endpoint was accepted for offline commissioning"
fi
[[ "$(shasum -a 256 "${config}" | awk '{print $1}')" == "${before}" ]] ||
  fail "failed commissioning changed the config"

if "${commissioner}" \
  --config "${config}" \
  --endpoint http://127.0.0.1:8005/mcp/ \
  --snapshot-id bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \
  --collection "ADF Doctrine" \
  --query "Joint Military Appreciation Process mission analysis" \
  --curl "${mock_curl}" >/dev/null 2>&1; then
  fail "snapshot mismatch was accepted"
fi
[[ "$(shasum -a 256 "${config}" | awk '{print $1}')" == "${before}" ]] ||
  fail "snapshot mismatch changed the config"

printf 'command RAG commissioning contract passed\n'
