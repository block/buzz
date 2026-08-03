#!/usr/bin/env bash
# Stop only relay/ACP processes whose markers were created by the Core pilot launcher.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PILOT_REPO_ROOT="$(cd "$script_dir/.." && pwd)"
# This is a repository-owned helper, not a user configuration file.
source "$script_dir/core-pilot-lib.sh"

pilot_parse_paths "$@"
PILOT_BIN_DIR="$PILOT_REPO_ROOT/target/release"
pilot_stop_marker "$PILOT_STATE_DIR/acp.pid" "$PILOT_BIN_DIR/buzz-acp"
pilot_stop_marker "$PILOT_STATE_DIR/relay.pid" "$PILOT_BIN_DIR/buzz-relay"
printf 'Core pilot processes stopped; Docker volumes and services were left intact.\n'
