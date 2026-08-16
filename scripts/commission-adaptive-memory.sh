#!/usr/bin/env bash
set -euo pipefail

runtime_root="${COMMAND_ADVISER_MEMORY_HOME:-${HOME}/Library/Application Support/Command Adviser/Memory}"
launch_agents_dir="${COMMAND_ADVISER_LAUNCH_AGENTS_DIR:-${HOME}/Library/LaunchAgents}"
source_root="${MEMORY_MCP_SOURCE:-${HOME}/Documents/Memory MCP/MemoryMCPServer}"
python_bin="${MEMORY_PYTHON:-/opt/homebrew/opt/python@3.11/bin/python3.11}"
launchctl_bin="${LAUNCHCTL:-launchctl}"
label="com.navigatorran.command-adviser-memory"
plist="${launch_agents_dir}/${label}.plist"
uid="$(id -u)"

[[ -f "${source_root}/pyproject.toml" ]] || {
  printf 'Memory MCP source was not found: %s\n' "${source_root}" >&2
  exit 2
}
[[ -x "${python_bin}" ]] || {
  printf 'Python 3.11 or newer was not found: %s\n' "${python_bin}" >&2
  exit 2
}

mkdir -p \
  "${runtime_root}/vault" \
  "${runtime_root}/index" \
  "${runtime_root}/logs" \
  "${launch_agents_dir}"

if [[ ! -x "${runtime_root}/venv/bin/python" ]]; then
  "${python_bin}" -m venv "${runtime_root}/venv"
fi
"${runtime_root}/venv/bin/python" -m pip install \
  --disable-pip-version-check \
  --quiet \
  --upgrade \
  "${source_root}"

xml_escape() {
  local value="$1"
  value="${value//&/&amp;}"
  value="${value//</&lt;}"
  value="${value//>/&gt;}"
  printf '%s' "${value}"
}

runtime_xml="$(xml_escape "${runtime_root}")"
cat >"${plist}" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${runtime_xml}/venv/bin/memory-mcp</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>MEMORY_HOST</key>
    <string>127.0.0.1</string>
    <key>MEMORY_PORT</key>
    <string>18006</string>
    <key>MEMORY_VAULT_ROOT</key>
    <string>${runtime_xml}/vault</string>
    <key>MEMORY_INDEX_ROOT</key>
    <string>${runtime_xml}/index</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>${runtime_xml}/logs/stdout.log</string>
  <key>StandardErrorPath</key>
  <string>${runtime_xml}/logs/stderr.log</string>
</dict>
</plist>
PLIST

plutil -lint "${plist}" >/dev/null
"${launchctl_bin}" bootout "gui/${uid}/${label}" >/dev/null 2>&1 || true
"${launchctl_bin}" bootstrap "gui/${uid}" "${plist}"
"${launchctl_bin}" kickstart -k "gui/${uid}/${label}"

printf 'Command Adviser Memory is installed at http://127.0.0.1:18006/mcp\n'
