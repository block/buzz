#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 <sidecar-directory> [target-triple]" >&2
  exit 64
fi

sidecar_dir=$1
target=${2:-}
sidecars=(buzz-acp buzz-agent buzz-lmstudio-agent buzz-dev-mcp git-credential-nostr buzz)
if [[ "${target}" != *windows* ]]; then
  sidecars+=(buzz-backend-kubernetes)
fi

if [[ ! -d "${sidecar_dir}" ]]; then
  echo "Error: sidecar directory does not exist: ${sidecar_dir}" >&2
  exit 1
fi

suffix=""
extension=""
requires_executable=true
if [[ -n "${target}" ]]; then
  suffix="-${target}"
fi
if [[ "${target}" == *windows* ]]; then
  extension=".exe"
  requires_executable=false
fi

failed=false
for sidecar in "${sidecars[@]}"; do
  path="${sidecar_dir}/${sidecar}${suffix}${extension}"
  if [[ ! -f "${path}" ]]; then
    echo "Error: missing sidecar: ${path}" >&2
    failed=true
    continue
  fi
  if [[ ! -s "${path}" ]]; then
    echo "Error: empty sidecar: ${path}" >&2
    failed=true
  fi
  if [[ "${requires_executable}" == true && ! -x "${path}" ]]; then
    echo "Error: non-executable sidecar: ${path}" >&2
    failed=true
  fi
done

if [[ "${failed}" == true ]]; then
  exit 1
fi

echo "Verified desktop sidecars in ${sidecar_dir}"
