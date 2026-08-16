#!/usr/bin/env bash
set -euo pipefail

config=""
endpoint=""
snapshot_id=""
collection=""
query=""
curl_bin="${CURL:-curl}"

usage() {
  printf 'usage: %s --config PATH --endpoint http://127.0.0.1:PORT/mcp/ --snapshot-id SHA256 --collection NAME --query TEXT [--curl PATH]\n' \
    "$(basename "$0")" >&2
}

while (($#)); do
  case "$1" in
    --config) config="${2:-}"; shift 2 ;;
    --endpoint) endpoint="${2:-}"; shift 2 ;;
    --snapshot-id) snapshot_id="${2:-}"; shift 2 ;;
    --collection) collection="${2:-}"; shift 2 ;;
    --query) query="${2:-}"; shift 2 ;;
    --curl) curl_bin="${2:-}"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) usage; exit 64 ;;
  esac
done

if [[ -z "${config}" || -z "${endpoint}" || -z "${snapshot_id}" ||
      -z "${collection}" || -z "${query}" ]]; then
  usage
  exit 64
fi
if [[ "${curl_bin}" == */* ]]; then
  [[ -x "${curl_bin}" ]] || { usage; exit 64; }
else
  curl_bin="$(command -v "${curl_bin}")" || { usage; exit 64; }
fi
if [[ ! "${endpoint}" =~ ^http://127\.0\.0\.1:([1-9][0-9]{0,4})/mcp/$ ]] ||
   ((10#${BASH_REMATCH[1]:-0} > 65535)); then
  printf '[command-rag] endpoint must be literal loopback http://127.0.0.1:PORT/mcp/\n' >&2
  exit 1
fi
if [[ ! "${snapshot_id}" =~ ^[0-9a-f]{64}$ ]]; then
  printf '[command-rag] snapshot ID must be a lowercase SHA-256 digest\n' >&2
  exit 1
fi
if [[ ! -f "${config}" || -L "${config}" || "$(stat -f %Lp "${config}")" != "600" ]]; then
  printf '[command-rag] trusted source config must be a protected regular file\n' >&2
  exit 1
fi
if ! jq -e '.schema_version == 1 and (.rag_url | type == "string")' "${config}" >/dev/null; then
  printf '[command-rag] trusted source config is invalid\n' >&2
  exit 1
fi

base_url="${endpoint%/mcp/}"
health="$(${curl_bin} --silent --show-error --fail --max-time 15 "${base_url}/health")"
if ! jq -e '.status == "ok" and (.points | type == "number") and .points > 0' \
  <<<"${health}" >/dev/null; then
  printf '[command-rag] local health check failed\n' >&2
  exit 1
fi

payload="$(jq -cn --arg query "${query}" --arg collection "${collection}" \
  '{query:$query,collections:[$collection],top_k:5,rerank:false}')"
search="$(${curl_bin} --silent --show-error --fail --max-time 45 \
  -H 'Content-Type: application/json' -d "${payload}" "${base_url}/search")"
if ! jq -e --arg snapshot "${snapshot_id}" --arg collection "${collection}" '
  .diagnostics.snapshot_id == $snapshot and
  (.results | type == "array" and length > 0) and
  any(.results[];
    (.point_id | type == "string" and length > 0) and
    (.doc_name | type == "string" and length > 0) and
    .collection == $collection and
    ((.page_no | type == "number") or
     (.chunk_idx | type == "number") or
     (.section_path | type == "array" and length > 0)))
' <<<"${search}" >/dev/null; then
  printf '[command-rag] semantic canary or snapshot identity failed\n' >&2
  exit 1
fi

temporary="$(mktemp "${config}.tmp.XXXXXX")"
cleanup() {
  rm -f "${temporary}"
}
trap cleanup EXIT
jq --arg endpoint "${endpoint}" '.rag_url = $endpoint' "${config}" >"${temporary}"
chmod 600 "${temporary}"
mv -f "${temporary}" "${config}"
trap - EXIT

point_id="$(jq -r --arg collection "${collection}" \
  '.results[] | select(.collection == $collection) | .point_id' <<<"${search}" | head -n 1)"
printf '[command-rag] commissioned snapshot %s at %s; semantic point %s\n' \
  "${snapshot_id}" "${endpoint}" "${point_id}"
