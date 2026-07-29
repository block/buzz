#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT
TARGET="aarch64-apple-darwin"

python3 - "${ROOT}/desktop/src-tauri/tauri.conf.json" <<'PY'
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    external = json.load(handle)["bundle"]["externalBin"]
assert "binaries/buzz-acp" in external
assert "binaries/buzz" in external
PY

grep -q 'for bin in buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz' "${ROOT}/Justfile"
grep -q 'for bin in buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz' "${ROOT}/.github/workflows/ci.yml"

mkdir -p "${TMP}/bin" "${TMP}/target/${TARGET}/release"
cat > "${TMP}/bin/rustc" <<'SH'
#!/usr/bin/env bash
printf '%s\n' 'host: aarch64-apple-darwin'
SH
chmod 0755 "${TMP}/bin/rustc"
for bin in buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr; do
    printf '#!/usr/bin/env bash\nexit 0\n' > "${TMP}/target/${TARGET}/release/${bin}"
    chmod 0755 "${TMP}/target/${TARGET}/release/${bin}"
done
cat > "${TMP}/target/${TARGET}/release/buzz" <<'SH'
#!/usr/bin/env bash
if [[ "${1:-}" == "__publication-fence-capability" ]]; then
    printf '%s\n' 'buzz-publication-fence-v1'
    exit 0
fi
exit 64
SH
chmod 0755 "${TMP}/target/${TARGET}/release/buzz"

cd "${TMP}"
PATH="${TMP}/bin:${PATH}" "${ROOT}/scripts/bundle-sidecars.sh" "${TARGET}" >/dev/null
ACP="desktop/src-tauri/binaries/buzz-acp-${TARGET}"
CLI="desktop/src-tauri/binaries/buzz-${TARGET}"
test -x "${ACP}"
test -x "${CLI}"
test "$("${CLI}" __publication-fence-capability)" = "buzz-publication-fence-v1"

echo "tauri sidecar capability set: PASS"
