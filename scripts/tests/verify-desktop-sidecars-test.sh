#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
verifier="${repo_root}/scripts/verify-desktop-sidecars.sh"
justfile="${repo_root}/Justfile"
test_tmp=$(mktemp -d)
trap 'rm -rf "${test_tmp}"' EXIT

if [[ ! -x "${verifier}" ]]; then
  echo "desktop sidecar verifier is missing or not executable: ${verifier}" >&2
  exit 1
fi

target="aarch64-apple-darwin"
sidecars=(
  buzz-acp
  buzz-agent
  buzz-lmstudio-agent
  buzz-dev-mcp
  git-credential-nostr
  buzz
  buzz-backend-kubernetes
)

for sidecar in "${sidecars[@]}"; do
  path="${test_tmp}/${sidecar}-${target}"
  printf '#!/usr/bin/env bash\nexit 0\n' >"${path}"
  chmod 755 "${path}"
done

"${verifier}" "${test_tmp}" "${target}" >/dev/null

zero_path="${test_tmp}/buzz-acp-${target}"
: >"${zero_path}"
zero_output="${test_tmp}/zero-output"
if "${verifier}" "${test_tmp}" "${target}" >"${zero_output}" 2>&1; then
  echo "verifier accepted a zero-byte sidecar" >&2
  exit 1
fi
grep -Fq "empty sidecar" "${zero_output}"

printf '#!/usr/bin/env bash\nexit 0\n' >"${zero_path}"
chmod 644 "${zero_path}"
mode_output="${test_tmp}/mode-output"
if "${verifier}" "${test_tmp}" "${target}" >"${mode_output}" 2>&1; then
  echo "verifier accepted a non-executable Unix sidecar" >&2
  exit 1
fi
grep -Fq "non-executable sidecar" "${mode_output}"

chmod 755 "${zero_path}"
rm "${test_tmp}/buzz-agent-${target}"
missing_output="${test_tmp}/missing-output"
if "${verifier}" "${test_tmp}" "${target}" >"${missing_output}" 2>&1; then
  echo "verifier accepted a missing sidecar" >&2
  exit 1
fi
grep -Fq "missing sidecar" "${missing_output}"

printf '#!/usr/bin/env bash\nexit 0\n' >"${test_tmp}/buzz-agent-${target}"
chmod 755 "${test_tmp}/buzz-agent-${target}"
rm "${test_tmp}/buzz-backend-kubernetes-${target}"
kubernetes_output="${test_tmp}/kubernetes-output"
if "${verifier}" "${test_tmp}" "${target}" >"${kubernetes_output}" 2>&1; then
  echo "verifier accepted a missing Kubernetes provider sidecar" >&2
  exit 1
fi
grep -Fq "buzz-backend-kubernetes" "${kubernetes_output}"

release_recipe=$(sed -n '/^desktop-release-build /,/^desktop-ci:/p' "${justfile}")
grep -Fq 'cargo build --release --target "$TARGET"' <<<"${release_recipe}"
grep -Fq './scripts/bundle-sidecars.sh "$TARGET"' <<<"${release_recipe}"
grep -Fq './scripts/verify-desktop-sidecars.sh' <<<"${release_recipe}"
grep -Fq 'APPLE_SIGNING_IDENTITY=- pnpm tauri build' <<<"${release_recipe}"
grep -Fq 'codesign --verify --deep --strict --verbose=2 "$APP_BUNDLE"' <<<"${release_recipe}"
if grep -Fq -- '-p buzz-lmstudio-agent' <<<"${release_recipe}"; then
  echo "release recipe treats the buzz-lmstudio-agent binary as a Cargo package" >&2
  exit 1
fi
if grep -Eq 'touch .*buzz-(acp|agent|dev-mcp|lmstudio-agent)' <<<"${release_recipe}"; then
  echo "release recipe still creates placeholder harness sidecars" >&2
  exit 1
fi

echo "desktop release sidecar checks passed"
