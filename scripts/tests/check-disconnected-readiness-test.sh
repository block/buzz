#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT

fail() {
  printf 'disconnected readiness test failed: %s\n' "$*" >&2
  exit 1
}

mkdir -p \
  "${temporary}/Command Adviser.app" \
  "${temporary}/models" \
  "${temporary}/rag" \
  "${temporary}/memory" \
  "${temporary}/bin" \
  "${temporary}/docs" \
  "${temporary}/skills/learned-abcdef123456"
printf app >"${temporary}/Command Adviser.app/payload"
printf model >"${temporary}/models/gemma.gguf"
printf embedding >"${temporary}/models/bge-m3.bin"
printf rag >"${temporary}/rag/manifest.json"
printf memory >"${temporary}/memory/vault.tar.gz.enc"
printf relay >"${temporary}/bin/buzz-relay"
printf recovery >"${temporary}/docs/recovery.md"
printf skill >"${temporary}/skills/learned-abcdef123456/SKILL.md"
printf version >"${temporary}/skills/learned-abcdef123456/.skill-version.json"

manifest="${temporary}/manifest.json"
python3 "${repo_root}/scripts/build-seagoing-manifest.py" \
  --output "${manifest}" \
  --component app command-adviser "${temporary}/Command Adviser.app" \
  --component model gemma "${temporary}/models/gemma.gguf" \
  --component embedding_model bge-m3 "${temporary}/models/bge-m3.bin" \
  --component rag_snapshot rag "${temporary}/rag/manifest.json" \
  --component memory_backup memory "${temporary}/memory/vault.tar.gz.enc" \
  --component relay relay "${temporary}/bin/buzz-relay" \
  --component recovery runbook "${temporary}/docs/recovery.md" >/dev/null

mock_codesign="${temporary}/codesign"
cat >"${mock_codesign}" <<'SH'
#!/usr/bin/env bash
[[ "${MOCK_CODESIGN_FAIL:-0}" != 1 ]]
SH
chmod +x "${mock_codesign}"

mock_model="${temporary}/model-check"
cat >"${mock_model}" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ "${MOCK_MODEL_FAIL:-0}" != 1 ]] || exit 4
report=""
while (($#)); do
  case "$1" in
    --report) report="$2"; shift 2 ;;
    *) shift ;;
  esac
done
printf '%s\n' '{"instanceId":"gemma4-26b-official","generationCapacity":1,"reasoning":"off","result":"pass"}' >"${report}"
SH
chmod +x "${mock_model}"

mock_curl="${temporary}/curl"
cat >"${mock_curl}" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
url="${*: -1}"
case "${url}" in
  http://127.0.0.1:3000/health)
    [[ "${MOCK_CURL_FAIL:-}" != relay ]] || exit 22
    printf ok
    ;;
  http://127.0.0.1:18006/mcp)
    [[ "${MOCK_CURL_FAIL:-}" != memory ]] || exit 22
    printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"serverInfo":{"name":"memory","version":"3.4.7"}}}'
    ;;
  http://127.0.0.1:8005/health)
    [[ "${MOCK_CURL_FAIL:-}" != rag-health ]] || exit 22
    printf '%s\n' '{"status":"ok","points":123502}'
    ;;
  http://127.0.0.1:8005/search)
    [[ "${MOCK_CURL_FAIL:-}" != rag-search ]] || exit 22
    printf '%s\n' '{"diagnostics":{"snapshot_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"results":[{"point_id":"point-1","doc_name":"ADFP 5.0.1.pdf","collection":"ADF Doctrine","page_no":15,"text":"Mission analysis"}]}'
    ;;
  *) exit 22 ;;
esac
SH
chmod +x "${mock_curl}"

mock_route="${temporary}/route"
cat >"${mock_route}" <<'SH'
#!/usr/bin/env bash
if [[ "${MOCK_ROUTE:-online}" == offline ]]; then
  exit 1
fi
if [[ "${MOCK_ROUTE:-online}" == offline-sparse ]]; then
  printf '%s\n' 'route to: default' 'interface: en0'
  exit 0
fi
if [[ "${MOCK_ROUTE:-online}" == offline-bsd ]]; then
  printf '%s\n' 'route: writing to routing socket: not in table' >&2
  exit 77
fi
if [[ "${MOCK_ROUTE:-online}" == error ]]; then
  exit 2
fi
printf '%s\n' 'gateway: 192.168.20.1' 'interface: en0'
SH
chmod +x "${mock_route}"

checker="${repo_root}/scripts/check-disconnected-readiness.sh"
common=(
  --manifest "${manifest}"
  --report "${temporary}/report.json"
  --app "${temporary}/Command Adviser.app"
  --rag-snapshot aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
  --rag-collection "ADF Doctrine"
  --rag-query "Joint Military Appreciation Process mission analysis"
  --skills-root "${temporary}/skills"
  --require-skills
  --recovery-reserve-bytes 1
)

run_checker() {
  env \
    CODESIGN="${mock_codesign}" \
    CURL="${mock_curl}" \
    ROUTE="${mock_route}" \
    OFFLINE_MODEL_CHECK="${mock_model}" \
    MOCK_ROUTE=offline \
    "$@" "${checker}" "${common[@]}"
}

run_checker >/dev/null || fail "complete fixture did not pass"
jq -e '
  .ready == true and .components_ready == true and
  .network.disconnected_observed == true and
  .components.model.instance_id == "gemma4-26b-official" and
  .components.rag.point_id == "point-1"
' "${temporary}/report.json" >/dev/null || fail "pass report is incomplete"

run_checker MOCK_ROUTE=offline-sparse >/dev/null ||
  fail "successful macOS route probe without an external gateway did not pass"
jq -e '.ready == true and .network.summary == "no_external_gateway"' \
  "${temporary}/report.json" >/dev/null || fail "missing external gateway was not explicit"

run_checker MOCK_ROUTE=offline-bsd >/dev/null ||
  fail "macOS BSD no-route response did not pass"
jq -e '.ready == true and .network.summary == "no_default_route"' \
  "${temporary}/report.json" >/dev/null || fail "BSD no-route response was not recognized"

if run_checker MOCK_CODESIGN_FAIL=1 >/dev/null 2>&1; then
  fail "bad app signature passed"
fi
if run_checker MOCK_MODEL_FAIL=1 >/dev/null 2>&1; then
  fail "failed model generation passed"
fi
if run_checker MOCK_CURL_FAIL=relay >/dev/null 2>&1; then
  fail "unavailable relay passed"
fi
if run_checker MOCK_CURL_FAIL=memory >/dev/null 2>&1; then
  fail "unavailable Memory passed"
fi
if run_checker MOCK_CURL_FAIL=rag-search >/dev/null 2>&1; then
  fail "failed RAG semantic canary passed"
fi
if run_checker DISCONNECTED_FREE_BYTES_OVERRIDE=0 >/dev/null 2>&1; then
  fail "low disk headroom passed"
fi

rm -f "${temporary}/skills/learned-abcdef123456/.skill-version.json"
if run_checker >/dev/null 2>&1; then
  fail "missing active skill projection passed"
fi
printf version >"${temporary}/skills/learned-abcdef123456/.skill-version.json"

env \
  CODESIGN="${mock_codesign}" \
  CURL="${mock_curl}" \
  ROUTE="${mock_route}" \
  OFFLINE_MODEL_CHECK="${mock_model}" \
  MOCK_ROUTE=online \
  "${checker}" "${common[@]}" >/dev/null 2>&1 &&
  fail "online route was reported disconnected-ready"
jq -e '.components_ready == true and .ready == false and .network.external_default_route == true' \
  "${temporary}/report.json" >/dev/null || fail "online preflight was not distinguished"

env \
  CODESIGN="${mock_codesign}" \
  CURL="${mock_curl}" \
  ROUTE="${mock_route}" \
  OFFLINE_MODEL_CHECK="${mock_model}" \
  MOCK_ROUTE=error \
  "${checker}" "${common[@]}" >/dev/null 2>&1 &&
  fail "failed route observation was reported disconnected-ready"
jq -e '.components_ready == true and .ready == false and .network.summary == "route_probe_failed"' \
  "${temporary}/report.json" >/dev/null || fail "route probe failure was not explicit"

mv "${temporary}/Command Adviser.app" "${temporary}/missing.app"
if run_checker >/dev/null 2>&1; then
  fail "missing installed app passed"
fi

printf 'disconnected readiness fixtures passed\n'
