#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
commissioner="${repo_root}/scripts/commission-adaptive-memory.sh"
test_tmp="$(mktemp -d)"
runtime_root="${test_tmp}/runtime"
launch_agents="${test_tmp}/LaunchAgents"
source_root="${test_tmp}/source"
command_log="${test_tmp}/commands.log"

cleanup() {
  rm -rf "${test_tmp}"
}
trap cleanup EXIT

fail() {
  printf 'commission-adaptive-memory test failed: %s\n' "$*" >&2
  exit 1
}

mkdir -p "${source_root}"
printf '[project]\nname="memory-mcp"\nversion="0.1.0"\n' >"${source_root}/pyproject.toml"

mock_python="${test_tmp}/python3.12"
printf '%s\n' \
  '#!/bin/bash' \
  'set -euo pipefail' \
  'printf "python %s\n" "$*" >>"${ADAPTIVE_MEMORY_MOCK_LOG}"' \
  'if [[ "$1" == "-m" && "$2" == "venv" ]]; then' \
  '  mkdir -p "$3/bin"' \
  '  printf "#!/bin/bash\nprintf '\''venv-python %%s\\n'\'' \"\$*\" >>\"${ADAPTIVE_MEMORY_MOCK_LOG}\"\n" >"$3/bin/python"' \
  '  printf "#!/bin/bash\nexit 0\n" >"$3/bin/memory-mcp"' \
  '  chmod +x "$3/bin/python" "$3/bin/memory-mcp"' \
  'fi' \
  >"${mock_python}"
chmod +x "${mock_python}"

mock_launchctl="${test_tmp}/launchctl"
printf '%s\n' \
  '#!/bin/bash' \
  'set -euo pipefail' \
  'printf "launchctl %s\n" "$*" >>"${ADAPTIVE_MEMORY_MOCK_LOG}"' \
  >"${mock_launchctl}"
chmod +x "${mock_launchctl}"

ADAPTIVE_MEMORY_MOCK_LOG="${command_log}" \
COMMAND_ADVISER_MEMORY_HOME="${runtime_root}" \
COMMAND_ADVISER_LAUNCH_AGENTS_DIR="${launch_agents}" \
MEMORY_MCP_SOURCE="${source_root}" \
MEMORY_PYTHON="${mock_python}" \
LAUNCHCTL="${mock_launchctl}" \
  /bin/bash "${commissioner}"

plist="${launch_agents}/com.navigatorran.command-adviser-memory.plist"
[[ -f "${plist}" ]] || fail "LaunchAgent was not written"
grep -Fq '<string>127.0.0.1</string>' "${plist}" || fail "service is not loopback-bound"
grep -Fq '<string>18006</string>' "${plist}" || fail "service does not use the local command port"
grep -Fq "${runtime_root}/vault" "${plist}" || fail "vault is not persistent"
grep -Fq "${runtime_root}/venv/bin/memory-mcp" "${plist}" || fail "installed runtime is not used"
grep -Fq 'python -m venv' "${command_log}" || fail "isolated runtime was not created"
grep -Fq 'venv-python -m pip install --disable-pip-version-check' "${command_log}" ||
  fail "Memory MCP package was not installed"
grep -Fq 'launchctl bootstrap' "${command_log}" || fail "LaunchAgent was not loaded"
grep -Fq 'launchctl kickstart -k' "${command_log}" || fail "LaunchAgent was not started"

printf 'commission-adaptive-memory contract passed\n'
