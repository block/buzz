#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

FAKE="${TMP}/sprig"
cat > "${FAKE}" <<'SH'
#!/usr/bin/env bash
name="$(basename "$0")"
if [[ "$name" == "buzz" && "${1:-}" == "__publication-fence-capability" ]]; then
    printf '%s\n' 'buzz-publication-fence-v1'
    exit 0
fi
if [[ "$name" == "sprig" && "${1:-}" == "--version" ]]; then
    printf '%s\n' 'sprig package-test'
    exit 0
fi
exit 64
SH
chmod 0755 "${FAKE}"

cd "${ROOT}"
SKIP_BUILD=1 \
SPRIG_BIN="${FAKE}" \
DIST_DIR="${TMP}/dist" \
ARCHIVE_BASENAME="sprig-package-test" \
./scripts/build-sprig.sh 0.1.0-test >/dev/null

mkdir "${TMP}/unpacked"
tar -xzf "${TMP}/dist/sprig-package-test.tar.gz" -C "${TMP}/unpacked"
test -L "${TMP}/unpacked/buzz"
test "$(readlink "${TMP}/unpacked/buzz")" = "sprig"
test "$("${TMP}/unpacked/buzz" __publication-fence-capability)" = "buzz-publication-fence-v1"
grep -q '"name":"buzz"' "${TMP}/unpacked/sprig.json"

echo "sprig package capability: PASS"
