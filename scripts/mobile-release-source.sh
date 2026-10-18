#!/usr/bin/env bash

# Validate an explicitly selected stable branch against the marketing version.
# main remains the default for releases without a stabilization branch.
require_mobile_release_source() {
  local version="$1" branch="$2"
  if [[ "$branch" == main ]]; then
    return 0
  fi
  if [[ "$branch" != "mobile-v${version%.*}" ]]; then
    echo "Source branch must be main or mobile-v${version%.*} for version $version" >&2
    return 1
  fi
}
