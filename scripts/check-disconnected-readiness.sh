#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
manifest=""
report=""
app="/Applications/Command Adviser.app"
rag_endpoint="http://127.0.0.1:8005"
rag_snapshot=""
rag_collection=""
rag_query=""
memory_endpoint="http://127.0.0.1:18006/mcp"
relay_endpoint="http://127.0.0.1:3000/health"
skills_root="${HOME}/.buzz/.agents/skills"
require_skills=0
recovery_reserve_bytes=0

curl_bin="${CURL:-curl}"
codesign_bin="${CODESIGN:-codesign}"
route_bin="${ROUTE:-route}"
python_bin="${PYTHON:-python3}"
model_check="${OFFLINE_MODEL_CHECK:-${repo_root}/scripts/check-offline-model.sh}"

usage() {
  printf 'usage: %s --manifest PATH --report PATH --rag-snapshot SHA256 --rag-collection NAME --rag-query TEXT [options]\n' "$(basename "$0")" >&2
}

while (($#)); do
  case "$1" in
    --manifest) manifest="${2:-}"; shift 2 ;;
    --report) report="${2:-}"; shift 2 ;;
    --app) app="${2:-}"; shift 2 ;;
    --rag-endpoint) rag_endpoint="${2:-}"; shift 2 ;;
    --rag-snapshot) rag_snapshot="${2:-}"; shift 2 ;;
    --rag-collection) rag_collection="${2:-}"; shift 2 ;;
    --rag-query) rag_query="${2:-}"; shift 2 ;;
    --memory-endpoint) memory_endpoint="${2:-}"; shift 2 ;;
    --relay-endpoint) relay_endpoint="${2:-}"; shift 2 ;;
    --skills-root) skills_root="${2:-}"; shift 2 ;;
    --require-skills) require_skills=1; shift ;;
    --recovery-reserve-bytes) recovery_reserve_bytes="${2:-}"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) usage; exit 64 ;;
  esac
done

if [[ -z "${manifest}" || -z "${report}" || -z "${rag_snapshot}" ||
      -z "${rag_collection}" || -z "${rag_query}" ]]; then
  usage
  exit 64
fi
if [[ ! "${rag_endpoint}" =~ ^http://127\.0\.0\.1:[1-9][0-9]{0,4}$ ||
      ! "${memory_endpoint}" =~ ^http://127\.0\.0\.1:[1-9][0-9]{0,4}/mcp$ ||
      ! "${relay_endpoint}" =~ ^http://127\.0\.0\.1:[1-9][0-9]{0,4}/health$ ||
      ! "${rag_snapshot}" =~ ^[0-9a-f]{64}$ ||
      ! "${recovery_reserve_bytes}" =~ ^[0-9]+$ ]]; then
  printf 'disconnected readiness requires literal loopback endpoints and valid identities\n' >&2
  exit 64
fi

temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT
components_dir="${temporary}/components"
mkdir -p "${components_dir}" "$(dirname "${report}")"
components_ready=1

pass_component() {
  local name="$1"
  shift
  jq -cn --arg status pass "$@" '{status:$status} + $ARGS.named' >"${components_dir}/${name}.json"
}

fail_component() {
  local name="$1"
  local reason="$2"
  components_ready=0
  jq -cn --arg status fail --arg reason "${reason}" \
    '{status:$status,reason:$reason}' >"${components_dir}/${name}.json"
}

if "${python_bin}" "${repo_root}/scripts/build-seagoing-manifest.py" \
    --verify-manifest "${manifest}" >"${temporary}/manifest-check.json" 2>/dev/null; then
  bundle_id="$(jq -r '.bundle_id' "${temporary}/manifest-check.json")"
  pass_component manifest --arg bundle_id "${bundle_id}"
else
  fail_component manifest manifest_invalid
fi

if [[ -e "${app}" ]] && "${codesign_bin}" --verify --deep --strict "${app}" >/dev/null 2>&1; then
  pass_component app --arg path "${app}"
else
  fail_component app app_missing_or_signature_invalid
fi

if "${model_check}" --report "${temporary}/model.json" >/dev/null 2>&1 &&
   jq -e '.result == "pass" and .instanceId == "gemma4-26b-official" and .generationCapacity == 1 and .reasoning == "off"' \
     "${temporary}/model.json" >/dev/null 2>&1; then
  instance_id="$(jq -r '.instanceId' "${temporary}/model.json")"
  pass_component model --arg instance_id "${instance_id}" --argjson generation_capacity 1
else
  fail_component model qualified_model_generation_failed
fi

relay_response="$(${curl_bin} --silent --show-error --fail --max-time 10 "${relay_endpoint}" 2>/dev/null || true)"
if [[ "${relay_response}" == "ok" ]] || jq -e '.status == "ok"' <<<"${relay_response}" >/dev/null 2>&1; then
  pass_component relay --arg endpoint "${relay_endpoint}"
else
  fail_component relay relay_unavailable
fi

memory_payload='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"command-adviser-readiness","version":"1"}}}'
memory_response="$(${curl_bin} --silent --show-error --fail --max-time 10 \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  --data "${memory_payload}" "${memory_endpoint}" 2>/dev/null || true)"
if jq -e '.result.serverInfo.name == "memory" and (.result.serverInfo.version | type == "string")' \
    <<<"${memory_response}" >/dev/null 2>&1; then
  memory_version="$(jq -r '.result.serverInfo.version' <<<"${memory_response}")"
  pass_component memory --arg endpoint "${memory_endpoint}" --arg version "${memory_version}"
else
  fail_component memory memory_unavailable
fi

rag_health="$(${curl_bin} --silent --show-error --fail --max-time 15 \
  "${rag_endpoint}/health" 2>/dev/null || true)"
rag_payload="$(jq -cn --arg query "${rag_query}" --arg collection "${rag_collection}" \
  '{query:$query,collections:[$collection],top_k:5,rerank:false}')"
rag_search="$(${curl_bin} --silent --show-error --fail --max-time 45 \
  -H 'Content-Type: application/json' --data "${rag_payload}" \
  "${rag_endpoint}/search" 2>/dev/null || true)"
if jq -e '.status == "ok" and ((.points // 1) > 0)' <<<"${rag_health}" >/dev/null 2>&1 &&
   jq -e --arg snapshot "${rag_snapshot}" --arg collection "${rag_collection}" '
     .diagnostics.snapshot_id == $snapshot and
     any(.results[];
       .collection == $collection and
       (.point_id | type == "string" and length > 0) and
       (.doc_name | type == "string" and length > 0) and
       (.text | type == "string" and length > 0) and
       ((.page_no | type == "number") or
        (.chunk_idx | type == "number") or
        (.section_path | type == "array" and length > 0)))
   ' <<<"${rag_search}" >/dev/null 2>&1; then
  point_id="$(jq -r --arg collection "${rag_collection}" '.results[] | select(.collection == $collection) | .point_id' <<<"${rag_search}" | head -n 1)"
  document="$(jq -r --arg collection "${rag_collection}" '.results[] | select(.collection == $collection) | .doc_name' <<<"${rag_search}" | head -n 1)"
  pass_component rag --arg snapshot_id "${rag_snapshot}" --arg point_id "${point_id}" --arg document "${document}"
else
  fail_component rag rag_semantic_canary_failed
fi

skill_count=0
if [[ -d "${skills_root}" ]]; then
  while IFS= read -r directory; do
    if [[ -f "${directory}/SKILL.md" && -f "${directory}/.skill-version.json" ]]; then
      skill_count=$((skill_count + 1))
    fi
  done < <(find "${skills_root}" -mindepth 1 -maxdepth 1 -type d -name 'learned-*' -print 2>/dev/null)
fi
if ((require_skills == 0 || skill_count > 0)); then
  pass_component skills --argjson active_projection_count "${skill_count}" --argjson required "${require_skills}"
else
  fail_component skills active_skill_projection_missing
fi

disk_json="$(${python_bin} - "${app}" <<'PY'
import json
from pathlib import Path
import shutil
import sys

path = Path(sys.argv[1])
probe = path if path.exists() else path.parent
usage = shutil.disk_usage(probe)
print(json.dumps({"total": usage.total, "free": usage.free}))
PY
)"
disk_total="$(jq -r '.total' <<<"${disk_json}")"
disk_free="${DISCONNECTED_FREE_BYTES_OVERRIDE:-$(jq -r '.free' <<<"${disk_json}")}"
disk_reserve=$((disk_total / 5 + recovery_reserve_bytes))
if ((disk_free >= disk_reserve)); then
  pass_component disk --argjson total_bytes "${disk_total}" --argjson free_bytes "${disk_free}" --argjson required_free_bytes "${disk_reserve}"
else
  fail_component disk insufficient_recovery_headroom
fi

external_default_route=false
route_summary="route_probe_failed"
route_output="$(${route_bin} -n get default 2>/dev/null)"
route_status=$?
if ((route_status == 1)); then
  route_summary="no_default_route"
elif ((route_status == 0)) && grep -Eq 'gateway:[[:space:]]*([^[:space:]]+)' <<<"${route_output}"; then
  gateway="$(sed -n 's/^[[:space:]]*gateway:[[:space:]]*//p' <<<"${route_output}" | head -n 1)"
  if [[ -n "${gateway}" && "${gateway}" != 127.* && "${gateway}" != "::1" ]]; then
    external_default_route=true
    route_summary="external_default_route_present"
  else
    route_summary="loopback_default_route_only"
  fi
fi
disconnected_observed=false
if [[ "${route_summary}" == "no_default_route" || "${route_summary}" == "loopback_default_route_only" ]]; then
  disconnected_observed=true
fi

components_json="$(jq -s 'reduce .[] as $item ({}; . + $item)' \
  <(for file in "${components_dir}"/*.json; do
      name="$(basename "${file}" .json)"
      jq -cn --arg name "${name}" --slurpfile value "${file}" '{($name):$value[0]}'
    done))"
ready=false
if ((components_ready == 1)) && [[ "${disconnected_observed}" == true ]]; then
  ready=true
fi

jq -n \
  --argjson ready "${ready}" \
  --argjson components_ready "$([[ ${components_ready} -eq 1 ]] && echo true || echo false)" \
  --argjson components "${components_json}" \
  --argjson external_default_route "${external_default_route}" \
  --argjson disconnected_observed "${disconnected_observed}" \
  --arg route_summary "${route_summary}" \
  '{
    schema_version: 1,
    ready: $ready,
    components_ready: $components_ready,
    network: {
      external_default_route: $external_default_route,
      disconnected_observed: $disconnected_observed,
      summary: $route_summary
    },
    components: $components
  }' >"${temporary}/report.json"
mv -f "${temporary}/report.json" "${report}"

jq -c '{ready,components_ready,network}' "${report}"
[[ "${ready}" == true ]]
