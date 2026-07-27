#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

APP_PATH="$DESKTOP_DIR/src-tauri/target/release/bundle/macos/Buzz.app"
INSTALL_PATH="${BUZZ_INSTALL_APP_PATH:-/Applications/Buzz.app}"
ENTITLEMENTS_PATH="$DESKTOP_DIR/src-tauri/Entitlements.plist"
IDENTITY_NAME="${BUZZ_LOCAL_CODESIGN_IDENTITY:-Buzz Local Code Signing}"
KEYCHAIN_PATH="${BUZZ_CODESIGN_KEYCHAIN:-}"
CREATE_IDENTITY=0
NO_INSTALL=0
DRY_RUN=0
STATUS_ONLY=0
SEARCH_LIST_CHANGED=0
ORIGINAL_KEYCHAINS=""

usage() {
  cat <<'USAGE'
Usage: desktop/scripts/install-local-macos-app.sh [options]

Signs a locally built Buzz.app with a stable local code-signing identity, then
installs it to /Applications/Buzz.app. This keeps macOS Keychain ACLs stable
across local rebuilds; ad-hoc signatures fall back to a cdhash requirement and
make "Always Allow" prompts come back after each rebuild.

Options:
  --app PATH             Buzz.app bundle to sign.
  --install-path PATH    Destination app path. Default: /Applications/Buzz.app
  --identity NAME        Code-signing identity common name.
                         Default: Buzz Local Code Signing
  --keychain PATH        Keychain containing/receiving the identity.
                         Default: login keychain
  --create-identity      Create a trusted local code-signing identity if absent.
  --no-install           Sign and verify APP_PATH only.
  --status               Verify the installed app and Keychain signing state.
  --dry-run              Print actions without mutating files/keychains.
  -h, --help             Show this help.

Examples:
  just desktop-install-local-macos --create-identity
  just desktop-signing-status
  desktop/scripts/install-local-macos-app.sh --no-install --app /tmp/Buzz.app
USAGE
}

log() {
  printf '==> %s\n' "$*"
}

warn() {
  printf 'warning: %s\n' "$*" >&2
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

print_cmd() {
  printf '+'
  local arg
  for arg in "$@"; do
    printf ' %q' "$arg"
  done
  printf '\n'
}

run() {
  if [[ "$DRY_RUN" == "1" ]]; then
    print_cmd "$@"
  else
    "$@"
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app)
      [[ $# -ge 2 ]] || die "--app requires a path"
      APP_PATH="$2"
      shift 2
      ;;
    --install-path)
      [[ $# -ge 2 ]] || die "--install-path requires a path"
      INSTALL_PATH="$2"
      shift 2
      ;;
    --identity)
      [[ $# -ge 2 ]] || die "--identity requires a name"
      IDENTITY_NAME="$2"
      shift 2
      ;;
    --keychain)
      [[ $# -ge 2 ]] || die "--keychain requires a path"
      KEYCHAIN_PATH="$2"
      shift 2
      ;;
    --create-identity)
      CREATE_IDENTITY=1
      shift
      ;;
    --no-install)
      NO_INSTALL=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --status|--verify-installed)
      STATUS_ONLY=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

# `defaults read` treats a relative app bundle path as a preferences domain,
# rather than an Info.plist path. Make an explicit --app path absolute before
# inspecting or signing the bundle.
if [[ "$STATUS_ONLY" != "1" ]]; then
  APP_PATH="$(cd "$(dirname "$APP_PATH")" && pwd)/$(basename "$APP_PATH")"
fi

[[ "$(uname -s)" == "Darwin" ]] || die "macOS is required"
command -v codesign >/dev/null 2>&1 || die "codesign not found"
command -v security >/dev/null 2>&1 || die "security not found"
if [[ "$STATUS_ONLY" != "1" ]]; then
  command -v openssl >/dev/null 2>&1 || die "openssl not found"
  command -v ditto >/dev/null 2>&1 || die "ditto not found"
fi

if [[ -z "$KEYCHAIN_PATH" ]]; then
  KEYCHAIN_PATH="$(security login-keychain | tr -d ' "')"
fi

[[ -n "$KEYCHAIN_PATH" ]] || die "could not resolve login keychain"
if [[ "$STATUS_ONLY" != "1" ]]; then
  [[ -d "$APP_PATH" ]] || die "missing app bundle: $APP_PATH"
  [[ -f "$APP_PATH/Contents/Info.plist" ]] || die "missing Info.plist in $APP_PATH"
  [[ -f "$ENTITLEMENTS_PATH" ]] || die "missing entitlements: $ENTITLEMENTS_PATH"

  BUNDLE_ID="$(defaults read "$APP_PATH/Contents/Info" CFBundleIdentifier 2>/dev/null || true)"
  [[ "$BUNDLE_ID" == "xyz.block.buzz.app" ]] || die "unexpected bundle id '$BUNDLE_ID'"
fi

capture_keychain_search_list() {
  ORIGINAL_KEYCHAINS="$(security list-keychains -d user | sed 's/^ *"//; s/"$//')"
}

restore_keychain_search_list() {
  [[ "$SEARCH_LIST_CHANGED" == "1" ]] || return 0
  [[ "$DRY_RUN" == "1" ]] && return 0
  [[ -n "$ORIGINAL_KEYCHAINS" ]] || return 0

  local -a keychains=()
  local keychain
  while IFS= read -r keychain; do
    [[ -n "$keychain" ]] && keychains+=("$keychain")
  done <<< "$ORIGINAL_KEYCHAINS"

  [[ "${#keychains[@]}" -gt 0 ]] || return 0
  security list-keychains -d user -s "${keychains[@]}" >/dev/null
}

ensure_keychain_in_search_list() {
  capture_keychain_search_list

  local found=0
  local -a keychains=("$KEYCHAIN_PATH")
  local keychain
  while IFS= read -r keychain; do
    [[ -z "$keychain" ]] && continue
    if [[ "$keychain" == "$KEYCHAIN_PATH" ]]; then
      found=1
      continue
    fi
    keychains+=("$keychain")
  done <<< "$ORIGINAL_KEYCHAINS"

  [[ "$found" == "1" ]] && return 0
  run security list-keychains -d user -s "${keychains[@]}"
  SEARCH_LIST_CHANGED=1
}

trap restore_keychain_search_list EXIT

find_identity_hash() {
  security find-identity -p codesigning -v 2>/dev/null |
    awk -v needle="\"$IDENTITY_NAME\"" 'index($0, needle) { print $2; exit }'
}

create_identity() {
  log "Creating local code-signing identity '$IDENTITY_NAME'"
  warn "macOS may ask for your keychain password. This is expected and not bypassed."

  if [[ "$DRY_RUN" == "1" ]]; then
    print_cmd security import "<generated-p12>" -k "$KEYCHAIN_PATH" -T /usr/bin/codesign
    print_cmd security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN_PATH" "<generated-cert>"
    return 0
  fi

  local tmpdir
  tmpdir="$(mktemp -d /tmp/buzz-local-codesign.XXXXXX)"
  local cert="$tmpdir/codesign.crt"
  local key="$tmpdir/codesign.key"
  local p12="$tmpdir/codesign.p12"
  local p12_password
  p12_password="$(openssl rand -hex 24)"

  openssl req -new -x509 -nodes -days 3650 -newkey rsa:2048 \
    -subj "/CN=$IDENTITY_NAME" \
    -addext "basicConstraints=critical,CA:true" \
    -addext "keyUsage=critical,digitalSignature,keyCertSign" \
    -addext "extendedKeyUsage=codeSigning" \
    -addext "subjectKeyIdentifier=hash" \
    -keyout "$key" \
    -out "$cert" >/dev/null 2>&1

  openssl pkcs12 -legacy -export \
    -name "$IDENTITY_NAME" \
    -inkey "$key" \
    -in "$cert" \
    -out "$p12" \
    -passout "pass:$p12_password" >/dev/null 2>&1

  security import "$p12" -k "$KEYCHAIN_PATH" -P "$p12_password" -T /usr/bin/codesign >/dev/null
  security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN_PATH" "$cert" >/dev/null

  chmod -R u+w "$tmpdir"
  rm -R "$tmpdir"
}

sign_nested_executables() {
  local identity_hash="$1"
  local executable

  while IFS= read -r executable; do
    log "Signing $(basename "$executable")"
    run codesign --force --options runtime --timestamp=none --sign "$identity_hash" "$executable"
  done < <(find "$APP_PATH/Contents/MacOS" -type f -perm -111 | sort)
}

sign_app_bundle() {
  local identity_hash="$1"

  log "Signing Buzz.app bundle"
  run codesign --force --deep --options runtime --timestamp=none \
    --entitlements "$ENTITLEMENTS_PATH" \
    --sign "$identity_hash" \
    "$APP_PATH"
}

designated_requirement() {
  codesign -d -r- "$1" 2>&1 | sed 's/^# //'
}

verify_signature() {
  local app="$1"
  local requirement

  log "Verifying signature for $app"
  run codesign --verify --deep --strict --verbose=2 "$app"

  if [[ "$DRY_RUN" == "1" ]]; then
    return 0
  fi

  requirement="$(designated_requirement "$app")"
  printf '%s\n' "$requirement"

  if grep -Fq 'designated => cdhash' <<< "$requirement"; then
    die "signature is still ad-hoc/cdhash based; Keychain prompts will not be stable"
  fi
  if ! grep -Fq 'identifier "xyz.block.buzz.app"' <<< "$requirement"; then
    die "designated requirement does not include the Buzz bundle identifier"
  fi
  if ! grep -Fq 'certificate leaf = H' <<< "$requirement"; then
    warn "designated requirement is not leaf-hash based; inspect before relying on Keychain ACL stability"
  fi
}

print_status() {
  local exit_code=0
  local bundle_id=""
  local requirement=""
  local leaf_hash=""

  printf 'install_path=%s\n' "$INSTALL_PATH"
  printf 'identity_name=%s\n' "$IDENTITY_NAME"

  if [[ ! -d "$INSTALL_PATH" ]]; then
    printf 'installed_app=missing\n'
    return 1
  fi

  bundle_id="$(defaults read "$INSTALL_PATH/Contents/Info" CFBundleIdentifier 2>/dev/null || true)"
  printf 'bundle_id=%s\n' "${bundle_id:-unknown}"
  if [[ "$bundle_id" != "xyz.block.buzz.app" ]]; then
    printf 'bundle_id_status=unexpected\n'
    exit_code=1
  else
    printf 'bundle_id_status=ok\n'
  fi

  if codesign --verify --deep --strict --verbose=2 "$INSTALL_PATH" >/dev/null 2>&1; then
    printf 'signature=valid\n'
  else
    printf 'signature=invalid\n'
    exit_code=1
  fi

  requirement="$(designated_requirement "$INSTALL_PATH" || true)"
  printf '%s\n' "$requirement"

  if grep -Fq 'designated => cdhash' <<< "$requirement"; then
    printf 'keychain_acl_stability=bad_cdhash\n'
    exit_code=1
  elif grep -Fq 'identifier "xyz.block.buzz.app"' <<< "$requirement" &&
    grep -Fq 'certificate leaf = H' <<< "$requirement"; then
    printf 'keychain_acl_stability=stable\n'
  else
    printf 'keychain_acl_stability=unknown\n'
    exit_code=1
  fi

  leaf_hash="$(sed -n 's/.*certificate leaf = H"\([^"]*\)".*/\1/p' <<< "$requirement" | head -n 1)"
  if [[ -n "$leaf_hash" ]] &&
    security find-identity -p codesigning -v 2>/dev/null | grep -iq "$leaf_hash"; then
    printf 'local_signing_identity=present\n'
  elif [[ -n "$(find_identity_hash || true)" ]]; then
    printf 'local_signing_identity=present_by_name\n'
  else
    printf 'local_signing_identity=missing\n'
    exit_code=1
  fi

  if security find-generic-password -s buzz-desktop >/dev/null 2>&1; then
    printf 'buzz_desktop_keychain_item=present\n'
  else
    printf 'buzz_desktop_keychain_item=missing\n'
  fi

  if pgrep -f "$INSTALL_PATH/Contents/MacOS/buzz-desktop" >/dev/null 2>&1; then
    printf 'running_from_install_path=yes\n'
  else
    printf 'running_from_install_path=no\n'
  fi

  return "$exit_code"
}

wait_for_buzz_to_quit() {
  local pattern="$INSTALL_PATH/Contents/MacOS/buzz-desktop"

  if ! pgrep -f "$pattern" >/dev/null 2>&1; then
    return 0
  fi

  log "Asking running Buzz to quit"
  /usr/bin/osascript -e 'tell application id "xyz.block.buzz.app" to quit' >/dev/null 2>&1 || true

  local _
  for _ in $(seq 1 20); do
    if ! pgrep -f "$pattern" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done

  die "Buzz is still running; quit it and rerun this script"
}

install_app() {
  local install_parent
  local backup_path
  install_parent="$(dirname "$INSTALL_PATH")"
  backup_path="$install_parent/Buzz.before-local-codesign-$(date +%Y%m%d-%H%M%S).app"

  [[ -d "$install_parent" ]] || die "install parent does not exist: $install_parent"

  if [[ "$DRY_RUN" == "1" ]]; then
    if [[ -e "$INSTALL_PATH" ]]; then
      print_cmd mv "$INSTALL_PATH" "$backup_path"
    fi
    print_cmd ditto "$APP_PATH" "$INSTALL_PATH"
    return 0
  fi

  wait_for_buzz_to_quit

  if [[ -e "$INSTALL_PATH" ]]; then
    log "Backing up existing app to $backup_path"
    mv "$INSTALL_PATH" "$backup_path"
  fi

  log "Installing signed app to $INSTALL_PATH"
  if ! ditto "$APP_PATH" "$INSTALL_PATH"; then
    if [[ -d "$backup_path" && ! -e "$INSTALL_PATH" ]]; then
      warn "Install failed; restoring previous app"
      mv "$backup_path" "$INSTALL_PATH"
    fi
    exit 1
  fi
}

if [[ "$STATUS_ONLY" == "1" ]]; then
  print_status
  exit $?
fi

ensure_keychain_in_search_list

IDENTITY_HASH="$(find_identity_hash || true)"
if [[ -z "$IDENTITY_HASH" ]]; then
  if [[ "$CREATE_IDENTITY" == "1" ]]; then
    create_identity
    IDENTITY_HASH="$(find_identity_hash || true)"
    if [[ -z "$IDENTITY_HASH" && "$DRY_RUN" == "1" ]]; then
      IDENTITY_HASH="DRYRUN-LOCAL-CODESIGN-IDENTITY"
    fi
  else
    die "missing code-signing identity '$IDENTITY_NAME'. Rerun with --create-identity once."
  fi
fi

[[ -n "$IDENTITY_HASH" ]] || die "could not create/find a valid code-signing identity"

log "Using code-signing identity $IDENTITY_NAME ($IDENTITY_HASH)"
sign_nested_executables "$IDENTITY_HASH"
sign_app_bundle "$IDENTITY_HASH"
verify_signature "$APP_PATH"

if [[ "$NO_INSTALL" == "1" ]]; then
  log "Signed app left in place: $APP_PATH"
  exit 0
fi

install_app
verify_signature "$INSTALL_PATH"
log "Done. The next Keychain prompt should need one final Always Allow for this stable identity."
