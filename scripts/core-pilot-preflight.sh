#!/usr/bin/env bash
# Validate a constrained Core local-pilot configuration without printing secrets.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PILOT_REPO_ROOT="$(cd "$script_dir/.." && pwd)"
# This is a repository-owned helper, not a user configuration file.
source "$script_dir/core-pilot-lib.sh"

pilot_parse_paths "$@"
pilot_load_and_validate
printf 'Core pilot preflight passed; constrained release stack is ready to start.\n'
