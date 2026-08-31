#!/bin/bash
# Build and install the exact immutable host runtimes trusted by buzz-acp.

set -euo pipefail
umask 077

readonly buzz_root="/Users/gabriel/.buzz/REPOS/buzz"
readonly gabe_root="/Users/gabriel/.openclaw"
readonly stacy_root="/Users/gabriel/stacy"
readonly trusted_build_root="/Users/gabriel/.buzz/REPOS/.trusted-build/buzz-acp"
readonly runtime_root="/Users/gabriel/.buzz/RUNTIME"
readonly node_source="/Users/gabriel/.nvm/versions/node/v24.13.1/bin/node"
readonly node_sha256="d36b3d980963d44bd2c5e844fac4cfeee26a167b744287a4e74a9575af9d0559"
readonly adapter_sha256="96a8efaf20cbc1cb92fb2ae2eca5a0bdefabba42f9cd6e2ca21299c724bd7c5c"

[[ $# -eq 4 ]] || {
  printf 'FATAL: usage: install-context-engine-runtimes.sh <buzz-commit> <gabe-commit> <stacy-commit> <buzz-acp-sha256>\n' >&2
  exit 1
}
readonly approved_buzz_commit="$1"
readonly approved_gabe_commit="$2"
readonly approved_stacy_commit="$3"
readonly buzz_acp_sha256="$4"
for digest in "$approved_buzz_commit:40" "$approved_gabe_commit:40" "$approved_stacy_commit:40" "$buzz_acp_sha256:64"; do
  value="${digest%:*}"
  length="${digest##*:}"
  [[ ${#value} -eq $length && "$value" =~ ^[0-9a-f]+$ ]] || {
    printf 'FATAL: approved install identity is malformed\n' >&2
    exit 1
  }
done
unset value length digest

run_git() {
  local repository="$1"
  shift
  /usr/bin/env -i \
    HOME=/var/empty \
    PATH=/usr/bin:/bin \
    GIT_CONFIG_NOSYSTEM=1 \
    GIT_CONFIG_SYSTEM=/dev/null \
    GIT_CONFIG_GLOBAL=/dev/null \
    GIT_ATTR_NOSYSTEM=1 \
    GIT_OPTIONAL_LOCKS=0 \
    /usr/bin/git \
      -c core.fsmonitor=false \
      -c core.hooksPath=/dev/null \
      -c core.untrackedCache=false \
      -c submodule.recurse=false \
      -C "$repository" "$@"
}

[[ "$(/usr/bin/uname -s)" == "Darwin" ]] || {
  printf 'FATAL: immutable runtime installation requires macOS\n' >&2
  exit 1
}

require_clean_main() {
  local repository="$1" name="$2" branch
  [[ -d "$repository" && ! -L "$repository" && "$(/bin/realpath "$repository")" == "$repository" ]] || {
    printf 'FATAL: %s source is not a canonical directory\n' "$name" >&2
    exit 1
  }
  branch="$(run_git "$repository" symbolic-ref --quiet --short HEAD)" || {
    printf 'FATAL: %s source is detached\n' "$name" >&2
    exit 1
  }
  [[ "$branch" == "main" ]] || {
    printf 'FATAL: %s source must be canonical main, not %s\n' "$name" "$branch" >&2
    exit 1
  }
  if run_git "$repository" ls-files -v | /usr/bin/grep -Eq '^[a-zS]'; then
    printf 'FATAL: %s source contains assume-unchanged or skip-worktree entries\n' "$name" >&2
    exit 1
  fi
  run_git "$repository" diff --quiet --no-ext-diff -- && \
    run_git "$repository" diff --cached --quiet --no-ext-diff -- || {
    printf 'FATAL: %s main has staged or unstaged tracked changes\n' "$name" >&2
    exit 1
  }
}

require_clean_main_pathspecs() {
  local repository="$1" name="$2" branch pathspec flags untracked tracked
  shift 2
  [[ -d "$repository" && ! -L "$repository" && "$(/bin/realpath "$repository")" == "$repository" ]] || {
    printf 'FATAL: %s source is not a canonical directory\n' "$name" >&2
    exit 1
  }
  branch="$(run_git "$repository" symbolic-ref --quiet --short HEAD)" || {
    printf 'FATAL: %s source is detached\n' "$name" >&2
    exit 1
  }
  [[ "$branch" == "main" ]] || {
    printf 'FATAL: %s source must be canonical main, not %s\n' "$name" "$branch" >&2
    exit 1
  }
  for pathspec in "$@"; do
    tracked="$(run_git "$repository" ls-files -- "$pathspec")"
    [[ -n "$tracked" ]] || {
      printf 'FATAL: %s required source is not tracked: %s\n' "$name" "$pathspec" >&2
      exit 1
    }
    flags="$(run_git "$repository" ls-files -v -- "$pathspec")"
    [[ -z "$flags" || -z "$(/usr/bin/printf '%s\n' "$flags" | /usr/bin/grep -E '^[a-zS]' || true)" ]] || {
      printf 'FATAL: %s required source has unsafe index flags: %s\n' "$name" "$pathspec" >&2
      exit 1
    }
    run_git "$repository" diff --quiet --no-ext-diff -- "$pathspec" && \
      run_git "$repository" diff --cached --quiet --no-ext-diff -- "$pathspec" || {
      printf 'FATAL: %s required source has staged or unstaged changes: %s\n' "$name" "$pathspec" >&2
      exit 1
    }
    untracked="$(run_git "$repository" ls-files --others --exclude-standard -- "$pathspec")"
    [[ -z "$untracked" ]] || {
      printf 'FATAL: %s required source contains untracked files: %s\n' "$name" "$pathspec" >&2
      exit 1
    }
  done
}

verify_sha256() {
  local path="$1" expected="$2" actual digest_output
  [[ -f "$path" && ! -L "$path" ]] || {
    printf 'FATAL: required regular file is unavailable: %s\n' "$path" >&2
    exit 1
  }
  [[ "$(/bin/realpath "$path")" == "$path" ]] || {
    printf 'FATAL: required file path is not canonical: %s\n' "$path" >&2
    exit 1
  }
  digest_output="$(/usr/bin/env -i HOME=/Users/gabriel PATH=/usr/bin:/bin /usr/bin/shasum -a 256 "$path")"
  actual="${digest_output%% *}"
  [[ "$actual" == "$expected" ]] || {
    printf 'FATAL: digest mismatch for %s\n' "$path" >&2
    exit 1
  }
}

ensure_private_directory() {
  local path="$1" parent expected_owner
  parent="$(/usr/bin/dirname "$path")"
  expected_owner="$(/usr/bin/id -u)"
  [[ -d "$parent" && ! -L "$parent" && "$(/bin/realpath "$parent")" == "$parent" ]] || {
    printf 'FATAL: private directory parent is unsafe: %s\n' "$parent" >&2
    exit 1
  }
  if [[ -e "$path" || -L "$path" ]]; then
    [[ -d "$path" && ! -L "$path" && "$(/bin/realpath "$path")" == "$path" ]] || {
      printf 'FATAL: private directory is unsafe: %s\n' "$path" >&2
      exit 1
    }
  else
    /bin/mkdir "$path"
  fi
  /bin/chmod 700 "$path"
  [[ "$(/usr/bin/stat -f '%u:%Lp' "$path")" == "$expected_owner:700" ]] || {
    printf 'FATAL: private directory owner or mode is unsafe: %s\n' "$path" >&2
    exit 1
  }
}

require_clean_main "$buzz_root" Buzz
require_clean_main_pathspecs "$gabe_root" Gabe \
  extensions/context-engine extensions/shared package.json package-lock.json tsconfig.json
require_clean_main_pathspecs "$stacy_root" Stacy \
  extensions/context-engine extensions/shared docker package.json package-lock.json tsconfig.json UPSTREAM_PIN

readonly gabe_adapter="$gabe_root/extensions/context-engine/scripts/gabe-acp.mjs"
readonly stacy_adapter="$stacy_root/extensions/context-engine/scripts/gabe-acp.mjs"

verify_sha256 "$node_source" "$node_sha256"
for adapter in "$gabe_adapter" "$stacy_adapter"; do
  verify_sha256 "$adapter" "$adapter_sha256"
done

# Building is deliberately outside this privileged installer. Running Cargo or
# Hermit here would consume user/repository tool configuration and turn a
# delayed ordinary-child write into unsandboxed code execution. The landing
# workflow supplies the reviewed `just trusted-buzz-acp-release` binary from a
# fixed checkout path; this installer accepts only its exact commit and digest.
[[ -d "$trusted_build_root" && ! -L "$trusted_build_root" && "$(/bin/realpath "$trusted_build_root")" == "$trusted_build_root" ]] || {
  printf 'FATAL: trusted buzz-acp build checkout is unsafe\n' >&2
  exit 1
}
[[ "$(run_git "$trusted_build_root" rev-parse HEAD)" == "$approved_buzz_commit" ]] || {
  printf 'FATAL: trusted buzz-acp build checkout is not at the approved Buzz commit\n' >&2
  exit 1
}
run_git "$trusted_build_root" diff --quiet --no-ext-diff -- && \
  run_git "$trusted_build_root" diff --cached --quiet --no-ext-diff -- || {
  printf 'FATAL: trusted buzz-acp build checkout has tracked changes\n' >&2
  exit 1
}
readonly buzz_acp_source="$trusted_build_root/target/release/buzz-acp"
verify_sha256 "$buzz_acp_source" "$buzz_acp_sha256"
[[ "$(/usr/bin/env -i HOME=/Users/gabriel PATH=/usr/bin:/bin /usr/bin/file -b "$buzz_acp_source")" == *"Mach-O 64-bit executable arm64"* ]] || {
  printf 'FATAL: buzz-acp is not the reviewed arm64 Mach-O runtime\n' >&2
  exit 1
}

buzz_commit="$(run_git "$buzz_root" rev-parse HEAD)"
gabe_commit="$(run_git "$gabe_root" rev-parse HEAD)"
stacy_commit="$(run_git "$stacy_root" rev-parse HEAD)"
[[ "$buzz_commit" == "$approved_buzz_commit" && "$gabe_commit" == "$approved_gabe_commit" && "$stacy_commit" == "$approved_stacy_commit" ]] || {
  printf 'FATAL: landed commits do not match the operator-approved install identities\n' >&2
  exit 1
}
stacy_upstream_pin="$(/usr/bin/env -i HOME=/var/empty PATH=/usr/bin:/bin /usr/bin/plutil -extract sha raw -expect string "$stacy_root/UPSTREAM_PIN" 2>/dev/null)" || {
  printf 'FATAL: Stacy UPSTREAM_PIN is missing or malformed\n' >&2
  exit 1
}
run_git "$gabe_root" cat-file -e "$stacy_upstream_pin^{commit}" 2>/dev/null && \
  run_git "$stacy_root" cat-file -e "$stacy_upstream_pin^{commit}" 2>/dev/null || {
  printf 'FATAL: Stacy UPSTREAM_PIN commit is unavailable in a canonical checkout\n' >&2
  exit 1
}
run_git "$gabe_root" merge-base --is-ancestor "$stacy_upstream_pin" "$gabe_commit" || {
  printf 'FATAL: Stacy UPSTREAM_PIN is not an ancestor of the landed Gabe commit\n' >&2
  exit 1
}
readonly -a shared_runtime_pathspecs=(
  extensions/context-engine
  extensions/shared
  package.json
  package-lock.json
  tsconfig.json
)
run_git "$gabe_root" diff --quiet --no-ext-diff "$stacy_upstream_pin" "$gabe_commit" -- "${shared_runtime_pathspecs[@]}" && \
  run_git "$stacy_root" diff --quiet --no-ext-diff "$stacy_upstream_pin" "$stacy_commit" -- "${shared_runtime_pathspecs[@]}" || {
  printf 'FATAL: landed Gabe or Stacy shared runtime bytes diverge from Stacy UPSTREAM_PIN\n' >&2
  exit 1
}
# The pin is the exact provenance of the shared runtime bytes. A later approved
# Gabe HEAD may differ only outside the guarded shared-runtime pathspecs above.
readonly receipt_root="$runtime_root/deployment-receipts/$buzz_commit-$stacy_upstream_pin-$stacy_commit"
readonly deployment_control_root="$runtime_root/deployment-control"
readonly install_lock="$deployment_control_root/install.lock"
readonly -a code_namespaces=(
  "$runtime_root/trusted-node"
  "$runtime_root/buzz-acp"
  "$runtime_root/context-engine"
  "$runtime_root/stacy-context-engine"
)

/bin/mkdir -p "$runtime_root"
[[ -d "$runtime_root" && ! -L "$runtime_root" && "$(/bin/realpath "$runtime_root")" == "$runtime_root" ]] || {
  printf 'FATAL: runtime root is not a canonical directory: %s\n' "$runtime_root" >&2
  exit 1
}
stage=""
declare -a installed_paths=()
install_complete=false
install_lock_held=false
install_lock_identity=""

require_context_engine_quiescence() {
  local current_uid process pids pid command
  current_uid="$(/usr/bin/id -u)"
  for process in Buzz buzz-desktop buzz-acp; do
    pids="$(/usr/bin/pgrep -U "$current_uid" -x "$process" 2>/dev/null || true)"
    [[ -z "$pids" ]] || {
      printf 'FATAL: Context Engine install requires %s to be stopped for uid %s (pids: %s)\n' \
        "$process" "$current_uid" "${pids//$'\n'/,}" >&2
      exit 1
    }
  done
  while read -r pid command; do
    [[ -n "${pid:-}" && -n "${command:-}" ]] || continue
    case "$command" in
      "$node_source $gabe_adapter"|"$node_source $gabe_adapter "*|\
      "$node_source $stacy_adapter"|"$node_source $stacy_adapter "*|\
      "$runtime_root"/trusted-node/*/node\ "$runtime_root"/context-engine/*/scripts/gabe-acp.mjs|\
      "$runtime_root"/trusted-node/*/node\ "$runtime_root"/context-engine/*/scripts/gabe-acp.mjs\ *|\
      "$runtime_root"/trusted-node/*/node\ "$runtime_root"/stacy-context-engine/*/scripts/gabe-acp.mjs|\
      "$runtime_root"/trusted-node/*/node\ "$runtime_root"/stacy-context-engine/*/scripts/gabe-acp.mjs\ *|\
      "$runtime_root"/buzz-acp/*/buzz-acp|"$runtime_root"/buzz-acp/*/buzz-acp\ *)
        printf 'FATAL: trusted Context Engine process is still running for uid %s (pid %s): %s\n' \
          "$current_uid" "$pid" "$command" >&2
        exit 1
        ;;
    esac
  done < <(/bin/ps -ww -U "$current_uid" -o pid= -o command=)
}

safe_remove_tree() {
  local path="$1"
  case "$path" in
    "$runtime_root"/.context-engine-install.*|\
    "$runtime_root"/trusted-node/"$node_sha256"|\
    "$runtime_root"/buzz-acp/"$buzz_acp_sha256"|\
    "$runtime_root"/context-engine/"$adapter_sha256"|\
    "$runtime_root"/stacy-context-engine/"$adapter_sha256"|\
    "$receipt_root") ;;
    *)
      printf 'FATAL: refusing cleanup outside this install transaction: %s\n' "$path" >&2
      return 1
      ;;
  esac
  [[ ! -e "$path" && ! -L "$path" ]] && return 0
  /usr/bin/chflags -R nouchg "$path" 2>/dev/null || true
  /bin/chmod -RN "$path" 2>/dev/null || true
  /bin/chmod -R u+rwX "$path" 2>/dev/null || true
  /bin/rm -rf "$path"
}

cleanup_install() {
  local exit_code="$?" index record path expected_identity current_identity current_lock_identity namespace
  if [[ "$install_complete" != "true" ]]; then
    for namespace in "${code_namespaces[@]}"; do
      if [[ -d "$namespace" && ! -L "$namespace" ]]; then
        /usr/bin/chflags nouchg "$namespace" 2>/dev/null || true
        /bin/chmod 755 "$namespace" 2>/dev/null || true
      fi
    done
    for ((index=${#installed_paths[@]} - 1; index >= 0; index--)); do
      record="${installed_paths[$index]}"
      path="${record%%|*}"
      expected_identity="${record#*|}"
      current_identity="$(/usr/bin/stat -f '%d:%i' "$path" 2>/dev/null || true)"
      if [[ "$current_identity" == "$expected_identity" ]]; then
        safe_remove_tree "$path" || true
      else
        printf 'WARNING: refusing rollback of replaced install path: %s\n' "$path" >&2
      fi
    done
  fi
  if [[ -n "${stage:-}" && "$stage" == "$runtime_root"/.context-engine-install.* ]]; then
    safe_remove_tree "$stage" || true
  fi
  for namespace in "${code_namespaces[@]}"; do
    if [[ -d "$namespace" && ! -L "$namespace" ]]; then
      freeze_runtime_namespace "$namespace" || \
        printf 'WARNING: failed to refreeze runtime namespace: %s\n' "$namespace" >&2
    fi
  done
  if [[ "$install_lock_held" == "true" ]]; then
    current_lock_identity="$(/usr/bin/stat -f '%d:%i' "$install_lock" 2>/dev/null || true)"
    if [[ "$current_lock_identity" == "$install_lock_identity" ]]; then
      /bin/rmdir "$install_lock" || true
    else
      printf 'WARNING: refusing to remove a replaced Context Engine install lock\n' >&2
    fi
  fi
  return "$exit_code"
}
trap cleanup_install EXIT

ensure_private_directory "$deployment_control_root"
/bin/mkdir "$install_lock" 2>/dev/null || {
  printf 'FATAL: another Context Engine runtime install is already in progress\n' >&2
  exit 1
}
install_lock_identity="$(/usr/bin/stat -f '%d:%i' "$install_lock")"
install_lock_held=true
[[ -d "$install_lock" && ! -L "$install_lock" && "$(/bin/realpath "$install_lock")" == "$install_lock" ]] || {
  printf 'FATAL: Context Engine install lock is unsafe\n' >&2
  exit 1
}
[[ "$(/usr/bin/stat -f '%u:%Lp' "$install_lock")" == "$(/usr/bin/id -u):700" ]] || {
  printf 'FATAL: Context Engine install lock owner or mode is unsafe\n' >&2
  exit 1
}
require_context_engine_quiescence
stage="$(/usr/bin/mktemp -d "$runtime_root/.context-engine-install.XXXXXX")"

tree_digest() {
  local root="$1" digest_output
  digest_output="$(
    (
      cd "$root"
      LC_ALL=C /usr/bin/find . -type f -print | LC_ALL=C /usr/bin/sort | while IFS= read -r relative; do
        /usr/bin/printf '%s\n' "$relative"
        /usr/bin/env -i HOME=/Users/gabriel PATH=/usr/bin:/bin /usr/bin/shasum -a 256 "$relative"
      done
    ) | /usr/bin/env -i HOME=/Users/gabriel PATH=/usr/bin:/bin /usr/bin/shasum -a 256
  )"
  /usr/bin/printf '%s' "${digest_output%% *}"
}

prepare_tree_modes() {
  local root="$1" kind="$2"
  case "$kind" in
    executable)
      /bin/chmod 555 "$root" "$root"/*
      ;;
    adapter)
      /bin/chmod 555 "$root" "$root/scripts"
      /bin/chmod 444 "$root/scripts/gabe-acp.mjs"
      ;;
    receipt)
      /bin/chmod 555 "$root"
      /bin/chmod 444 "$root/context-engine-runtimes.json"
      ;;
    *)
      printf 'FATAL: unknown runtime tree kind: %s\n' "$kind" >&2
      exit 1
      ;;
  esac
}

verify_frozen_tree() {
  local root="$1" kind="$2" path mode flags owner expected_owner
  expected_owner="$(/usr/bin/id -u)"
  [[ -d "$root" && ! -L "$root" ]] || return 1
  [[ -z "$(/usr/bin/find "$root" -type l -print -quit)" ]] || return 1
  while IFS= read -r path; do
    [[ ! -L "$path" ]] || return 1
    owner="$(/usr/bin/stat -f '%u' "$path")"
    [[ "$owner" == "$expected_owner" ]] || return 1
    mode="$(/usr/bin/stat -f '%Lp' "$path")"
    [[ "$mode" == "555" ]] || return 1
    flags="$(/usr/bin/stat -f '%Sf' "$path")"
    [[ ",$flags," == *,uchg,* || "$flags" == "uchg" ]] || return 1
  done < <(/usr/bin/find "$root" -type d -print)

  while IFS= read -r path; do
    [[ -f "$path" && ! -L "$path" ]] || return 1
    owner="$(/usr/bin/stat -f '%u' "$path")"
    [[ "$owner" == "$expected_owner" ]] || return 1
    mode="$(/usr/bin/stat -f '%Lp' "$path")"
    if [[ "$kind" == "executable" ]]; then
      [[ "$mode" == "555" ]] || return 1
    else
      [[ "$mode" == "444" ]] || return 1
    fi
    flags="$(/usr/bin/stat -f '%Sf' "$path")"
    [[ ",$flags," == *,uchg,* || "$flags" == "uchg" ]] || return 1
  done < <(/usr/bin/find "$root" -type f -print)
}

verify_frozen_namespace() {
  local namespace="$1" expected_owner mode flags
  expected_owner="$(/usr/bin/id -u)"
  [[ -d "$namespace" && ! -L "$namespace" && "$(/bin/realpath "$namespace")" == "$namespace" ]] || return 1
  mode="$(/usr/bin/stat -f '%Lp' "$namespace")"
  flags="$(/usr/bin/stat -f '%Sf' "$namespace")"
  [[ "$(/usr/bin/stat -f '%u' "$namespace")" == "$expected_owner" && "$mode" == "555" ]] || return 1
  [[ ",$flags," == *,uchg,* || "$flags" == "uchg" ]]
}

prepare_runtime_namespace() {
  local namespace="$1"
  if [[ -e "$namespace" || -L "$namespace" ]]; then
    verify_frozen_namespace "$namespace" || {
      printf 'FATAL: existing runtime namespace is not frozen and trusted: %s\n' "$namespace" >&2
      exit 1
    }
    /usr/bin/chflags nouchg "$namespace"
  else
    /bin/mkdir "$namespace"
  fi
  /bin/chmod 755 "$namespace"
  [[ -d "$namespace" && ! -L "$namespace" && "$(/bin/realpath "$namespace")" == "$namespace" ]] || {
    printf 'FATAL: runtime namespace is unsafe: %s\n' "$namespace" >&2
    exit 1
  }
  [[ "$(/usr/bin/stat -f '%u:%Lp' "$namespace")" == "$(/usr/bin/id -u):755" ]] || {
    printf 'FATAL: runtime namespace owner or mode is unsafe: %s\n' "$namespace" >&2
    exit 1
  }
}

freeze_runtime_namespace() {
  local namespace="$1"
  /bin/chmod 555 "$namespace"
  /usr/bin/chflags uchg "$namespace"
  verify_frozen_namespace "$namespace"
}

install_or_reuse_tree() {
  local staged="$1" destination="$2" kind="$3" parent staged_digest staged_identity installed_digest installed_identity
  prepare_tree_modes "$staged" "$kind"
  [[ -z "$(/usr/bin/find "$staged" -type l -print -quit)" ]] || {
    printf 'FATAL: staged runtime contains a symlink: %s\n' "$staged" >&2
    exit 1
  }
  staged_digest="$(tree_digest "$staged")"
  staged_identity="$(/usr/bin/stat -f '%d:%i' "$staged")"
  if [[ -e "$destination" || -L "$destination" ]]; then
    [[ -d "$destination" && ! -L "$destination" ]] || {
      printf 'FATAL: immutable destination is not a regular directory: %s\n' "$destination" >&2
      exit 1
    }
    [[ "$(/bin/realpath "$destination")" == "$destination" ]] || {
      printf 'FATAL: immutable destination is not canonical: %s\n' "$destination" >&2
      exit 1
    }
    /usr/bin/env -i HOME=/Users/gabriel PATH=/usr/bin:/bin /usr/bin/diff -qr "$staged" "$destination" >/dev/null || {
      printf 'FATAL: existing immutable destination has different content: %s\n' "$destination" >&2
      exit 1
    }
    verify_frozen_tree "$destination" "$kind" || {
      printf 'FATAL: existing immutable destination has unsafe metadata: %s\n' "$destination" >&2
      exit 1
    }
    safe_remove_tree "$staged"
    return 0
  fi

  parent="$(/usr/bin/dirname "$destination")"
  /bin/mkdir -p "$parent"
  [[ -d "$parent" && ! -L "$parent" && "$(/bin/realpath "$parent")" == "$parent" ]] || {
    printf 'FATAL: immutable destination parent is unsafe: %s\n' "$parent" >&2
    exit 1
  }
  /bin/mv -n "$staged" "$destination"
  [[ ! -e "$staged" && ! -L "$staged" ]] || {
    printf 'FATAL: immutable destination appeared during install: %s\n' "$destination" >&2
    exit 1
  }
  installed_identity="$(/usr/bin/stat -f '%d:%i' "$destination")"
  [[ "$installed_identity" == "$staged_identity" ]] || {
    printf 'FATAL: immutable destination identity changed during install: %s\n' "$destination" >&2
    exit 1
  }
  installed_paths+=("$destination|$installed_identity")
  /usr/bin/chflags -R uchg "$destination"
  installed_digest="$(tree_digest "$destination")"
  [[ "$installed_digest" == "$staged_digest" ]] || {
    printf 'FATAL: installed runtime content changed during promotion: %s\n' "$destination" >&2
    exit 1
  }
  verify_frozen_tree "$destination" "$kind" || {
    printf 'FATAL: installed runtime failed frozen metadata verification: %s\n' "$destination" >&2
    exit 1
  }
}

stage_context_runtime() {
  local source_root="$1" destination_name="$2" staged_root
  staged_root="$stage/$destination_name"
  /bin/mkdir -p "$staged_root/scripts"
  /bin/cp "$source_root/extensions/context-engine/scripts/gabe-acp.mjs" "$staged_root/scripts/gabe-acp.mjs"
  verify_sha256 "$staged_root/scripts/gabe-acp.mjs" "$adapter_sha256"
}

stage_context_runtime "$gabe_root" gabe-context
stage_context_runtime "$stacy_root" stacy-context

/bin/mkdir -p "$stage/trusted-node" "$stage/buzz-acp"
/bin/cp "$node_source" "$stage/trusted-node/node"
/bin/cp "$buzz_acp_source" "$stage/buzz-acp/buzz-acp"
verify_sha256 "$stage/trusted-node/node" "$node_sha256"
verify_sha256 "$stage/buzz-acp/buzz-acp" "$buzz_acp_sha256"

/bin/mkdir -p "$stage/receipt"
/usr/bin/printf '%s\n' \
  "{\"buzzCommit\":\"$buzz_commit\",\"gabeCommit\":\"$stacy_upstream_pin\",\"stacyCommit\":\"$stacy_commit\",\"nodeSha256\":\"$node_sha256\",\"buzzAcpSha256\":\"$buzz_acp_sha256\",\"adapterSha256\":\"$adapter_sha256\"}" \
  >"$stage/receipt/context-engine-runtimes.json"

require_context_engine_quiescence
for namespace in "${code_namespaces[@]}"; do
  prepare_runtime_namespace "$namespace"
done
install_or_reuse_tree "$stage/trusted-node" "$runtime_root/trusted-node/$node_sha256" executable
install_or_reuse_tree "$stage/buzz-acp" "$runtime_root/buzz-acp/$buzz_acp_sha256" executable
install_or_reuse_tree "$stage/gabe-context" "$runtime_root/context-engine/$adapter_sha256" adapter
install_or_reuse_tree "$stage/stacy-context" "$runtime_root/stacy-context-engine/$adapter_sha256" adapter
install_or_reuse_tree "$stage/receipt" "$receipt_root" receipt

# Stacy's gateway writes only the opaque adapter capability into this private,
# non-immutable directory. The adapter is transport-only and never writes here.
readonly stacy_runtime_parent="$runtime_root/stacy-context-engine"
readonly stacy_runtime_home="$stacy_runtime_parent/home"
readonly stacy_credentials="$stacy_runtime_home/credentials"
[[ -d "$stacy_runtime_parent" && ! -L "$stacy_runtime_parent" && "$(/bin/realpath "$stacy_runtime_parent")" == "$stacy_runtime_parent" ]] || {
  printf 'FATAL: Stacy runtime parent is unsafe\n' >&2
  exit 1
}
if [[ -e "$stacy_runtime_home" || -L "$stacy_runtime_home" ]]; then
  [[ -d "$stacy_runtime_home" && ! -L "$stacy_runtime_home" && "$(/bin/realpath "$stacy_runtime_home")" == "$stacy_runtime_home" ]] || {
    printf 'FATAL: Stacy runtime home is unsafe\n' >&2
    exit 1
  }
else
  /bin/mkdir "$stacy_runtime_home"
fi
/bin/chmod 700 "$stacy_runtime_home"
[[ "$(/usr/bin/stat -f '%u:%Lp' "$stacy_runtime_home")" == "$(/usr/bin/id -u):700" ]] || {
  printf 'FATAL: Stacy runtime home owner or mode is unsafe\n' >&2
  exit 1
}
ensure_private_directory "$stacy_credentials"

# Durable prepared replies must exist before either trusted context engine can
# accept traffic. Create every new component one level at a time so an existing
# symlink can never redirect the installer outside the reviewed state roots.
readonly gabe_state_root="$gabe_root/state"
[[ -d "$gabe_state_root" && ! -L "$gabe_state_root" && "$(/bin/realpath "$gabe_state_root")" == "$gabe_state_root" ]] || {
  printf 'FATAL: Gabe state root is unsafe\n' >&2
  exit 1
}
ensure_private_directory "$gabe_state_root/context-engine-buzz"
ensure_private_directory "$gabe_state_root/context-engine-buzz/prepared"
ensure_private_directory "$stacy_runtime_home/state"
ensure_private_directory "$stacy_runtime_home/state/context-engine-buzz"
ensure_private_directory "$stacy_runtime_home/state/context-engine-buzz/prepared"

for namespace in "${code_namespaces[@]}"; do
  freeze_runtime_namespace "$namespace" || {
    printf 'FATAL: installed runtime namespace failed frozen verification: %s\n' "$namespace" >&2
    exit 1
  }
done

install_complete=true
printf 'Installed immutable Gabe and Stacy Buzz context runtimes.\n'
