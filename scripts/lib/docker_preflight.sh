# shellcheck shell=bash
# Shared Docker preflight helpers for local setup scripts.

# Classify `docker info` stderr. Prints one of: ok | permission | not_running | other
# Usage: docker_preflight_classify "<stderr text>" <exit_code>
docker_preflight_classify() {
  local err="${1-}"
  local code="${2:-1}"
  if [[ "$code" -eq 0 ]]; then
    printf 'ok\n'
    return 0
  fi
  if printf '%s' "$err" | grep -qiE 'permission denied|dial unix .*: connect: permission denied'; then
    printf 'permission\n'
    return 0
  fi
  if printf '%s' "$err" | grep -qiE 'Cannot connect|Is the docker daemon running|connection refused'; then
    printf 'not_running\n'
    return 0
  fi
  printf 'other\n'
}

# Print a human-actionable error for a failed `docker info` and return 1.
docker_preflight_or_die() {
  local docker_info_err code
  set +e
  docker_info_err="$(docker info 2>&1)"
  code=$?
  set -e

  case "$(docker_preflight_classify "$docker_info_err" "$code")" in
    ok)
      return 0
      ;;
    permission)
      error "Docker is installed but this user cannot talk to the daemon (permission denied)."
      error "On Linux: add your user to the docker group, then re-login:"
      error "  sudo usermod -aG docker \"\$USER\" && newgrp docker"
      error "Or run rootless Docker / start Docker Desktop and ensure your user can access it."
      return 1
      ;;
    not_running)
      error "Docker daemon is not running. Start Docker Desktop (or your engine) and try again."
      return 1
      ;;
    *)
      error "Docker is unreachable:"
      error "$docker_info_err"
      return 1
      ;;
  esac
}
