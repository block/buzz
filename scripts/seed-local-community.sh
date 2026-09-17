#!/usr/bin/env bash
# Seed local dev host -> community rows for row-zero host binding.
#
# The relay intentionally fails closed when the request Host header is not in
# `communities`. Local dev uses loopback hosts, so bootstrap must create those
# rows after migrations before desktop/Tauri HTTP bridge calls can succeed.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

if [[ -f ".env" ]]; then
  set -o allexport
  # shellcheck disable=SC1091
  source .env
  set +o allexport
fi

export PGHOST="${PGHOST:-localhost}"
export PGPORT="${PGPORT:-5432}"
export PGUSER="${PGUSER:-buzz}"
export PGPASSWORD="${PGPASSWORD:-buzz_dev}"
export PGDATABASE="${PGDATABASE:-buzz}"
export RELAY_URL="${RELAY_URL:-ws://localhost:3000}"

PYTHON_CMD=""
if python3 -c "import sys" &>/dev/null; then
  PYTHON_CMD="python3"
elif python -c "import sys" &>/dev/null; then
  PYTHON_CMD="python"
fi

if [[ -n "${PYTHON_CMD}" ]]; then
  hosts_sql=$(${PYTHON_CMD} - <<'PY'
import os
from urllib.parse import urlparse

relay_url = os.environ.get("RELAY_URL", "ws://localhost:3000")
parsed = urlparse(relay_url)

host = (parsed.hostname or "").rstrip(".").lower()
port = parsed.port
scheme = parsed.scheme.lower()

def authority(host, port, scheme):
    if not host:
        return ""
    display_host = f"[{host}]" if ":" in host and not host.startswith("[") else host
    default_port = (scheme == "ws" and port == 80) or (scheme == "wss" and port == 443)
    if port and not default_port:
        return f"{display_host}:{port}"
    return display_host

primary = authority(host, port, scheme)
hosts = []
if primary:
    hosts.append(primary)

# Local desktop/dev tooling has historically used both localhost and 127.0.0.1,
# and some HTTP clients can omit the default/non-default port in Host handling.
# Under row-zero host binding these are distinct hosts, so seed loopback aliases
# for local dev to avoid a fail-closed 404 when one side uses an alternate
# authority. Non-loopback deployments seed only RELAY_URL's authority.
if host in {"localhost", "127.0.0.1"}:
    hosts.extend(["localhost", "127.0.0.1"])
    if port:
        hosts.extend([f"localhost:{port}", f"127.0.0.1:{port}"])

seen = []
for h in hosts:
    if h and h not in seen:
        seen.append(h)

if not seen:
    raise SystemExit("could not derive a host from RELAY_URL")

lines = []
for h in seen:
    escaped = h.replace(chr(39), chr(39) * 2)
    lines.append(f"    ('{escaped}')")
print(",\n".join(lines))
PY
)
elif command -v node &>/dev/null; then
  hosts_sql=$(node -e "
const { URL } = require('url');
const relayUrl = process.env.RELAY_URL || 'ws://localhost:3000';
let parsed;
try { parsed = new URL(relayUrl); } catch (e) { parsed = new URL('ws://localhost:3000'); }
let host = (parsed.hostname || '').replace(/\.$/, '').toLowerCase();
let port = parsed.port;
let scheme = parsed.protocol ? parsed.protocol.replace(':', '').toLowerCase() : 'ws';

function authority(h, p, s) {
  if (!h) return '';
  let displayHost = (h.includes(':') && !h.startsWith('[')) ? '[' + h + ']' : h;
  let defaultPort = (s === 'ws' && p === '80') || (s === 'wss' && p === '443');
  if (p && !defaultPort) return displayHost + ':' + p;
  return displayHost;
}

let primary = authority(host, port, scheme);
let hosts = [];
if (primary) hosts.push(primary);

if (host === 'localhost' || host === '127.0.0.1') {
  hosts.push('localhost', '127.0.0.1');
  if (port) {
    hosts.push('localhost:' + port, '127.0.0.1:' + port);
  }
}

let seen = [];
for (let h of hosts) {
  if (h && !seen.includes(h)) seen.push(h);
}

if (seen.length === 0) {
  console.error('could not derive a host from RELAY_URL');
  process.exit(1);
}

let lines = seen.map(h => '    (\'' + h.replace(/'/g, '\'\'') + '\')');
console.log(lines.join(',\n'));
")
else
  echo "error: neither python nor node is available to parse RELAY_URL" >&2
  exit 1
fi

sql="
INSERT INTO communities (host)
SELECT host
FROM (VALUES
${hosts_sql}
) AS v(host)
ON CONFLICT (lower(host)) DO NOTHING;
"

if command -v psql >/dev/null 2>&1; then
  PGPASSWORD="${PGPASSWORD}" psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${PGDATABASE}" -v ON_ERROR_STOP=1 -c "${sql}"
elif docker exec buzz-postgres psql --version >/dev/null 2>&1; then
  docker exec -i -e PGPASSWORD="${PGPASSWORD}" buzz-postgres \
    psql -U "${PGUSER}" -d "${PGDATABASE}" -v ON_ERROR_STOP=1 -c "${sql}"
else
  echo "error: neither psql nor buzz-postgres docker psql is available" >&2
  exit 1
fi

echo "Seeded local dev community host(s):"
echo "${hosts_sql}" | sed -E "s/^ +\('(.+)'\),?$/  - \1/"