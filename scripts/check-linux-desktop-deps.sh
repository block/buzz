#!/usr/bin/env bash
# =============================================================================
# check-linux-desktop-deps.sh — preflight for the desktop app's system libs
# =============================================================================
# Usage: ./scripts/check-linux-desktop-deps.sh [--check|--install]
#
# The Tauri desktop crate links against system GTK/WebKitGTK libraries through
# `*-sys` crates whose build scripts run pkg-config. Hermit pins language
# toolchains, not system libraries, so a machine that has never had these
# installed fails hundreds of crates into a build with a cryptic:
#
#   pkg-config exited with status code 1
#   Package 'gdk-3.0' not found
#
# This preflight runs before desktop builds and turns that into one actionable
# message naming every missing module and the exact install command for the
# detected distribution. It is a no-op off Linux.
#
#   --check    (default) verify the modules; non-zero when something is missing.
#   --install  install the distro packages (uses sudo when not root), then
#              re-verify. Only supported on Debian/Ubuntu, Fedora/RHEL,
#              Arch, and openSUSE.
# =============================================================================
set -euo pipefail

MODE="check"
case "${1:-}" in
  "" | --check) MODE="check" ;;
  --install) MODE="install" ;;
  -h | --help)
    # Print the leading comment block (everything between the shebang and the
    # first statement) without hard-coding a line range that drifts.
    awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"
    exit 0
    ;;
  *)
    echo "usage: $0 [--check|--install]" >&2
    exit 2
    ;;
esac

# Test seams: the regression test overrides these instead of touching the host.
UNAME_S="${UNAME_S:-$(uname -s)}"
OS_RELEASE="${OS_RELEASE:-/etc/os-release}"
PKG_CONFIG="${PKG_CONFIG:-pkg-config}"

# Only Linux needs this preflight (macOS/Windows get their SDKs elsewhere).
if [[ "${UNAME_S}" != "Linux" ]]; then
  exit 0
fi

# pkg-config module names required by the desktop crate's `*-sys` dependencies.
# gtk+-3.0 pulls atk/cairo/gdk-pixbuf/pango through its Requires chain, and
# webkit2gtk-4.1 pulls javascriptcoregtk-4.1/libsoup-3.0 the same way; they are
# listed explicitly anyway so a partial installation names the exact missing
# library instead of just the top-level one.
MODULES=(
  gtk+-3.0
  webkit2gtk-4.1
  javascriptcoregtk-4.1
  libsoup-3.0
  alsa
  openssl
  librsvg-2.0
  ayatana-appindicator3-0.1
)

# Package names per distribution family. Keep this list aligned with the
# "Install Tauri dependencies (Linux)" steps in .github/workflows/.
APT_PACKAGES=(
  build-essential curl file libasound2-dev libayatana-appindicator3-dev
  libgtk-3-dev librsvg2-dev libssl-dev libwebkit2gtk-4.1-dev libxdo-dev
  patchelf pkg-config wget
)
DNF_PACKAGES=(
  alsa-lib-devel curl file gcc gcc-c++ libayatana-appindicator-gtk3-devel
  libxdo-devel make openssl-devel patchelf pkgconf-pkg-config
  webkit2gtk4.1-devel wget gtk3-devel librsvg2-devel
)
PACMAN_PACKAGES=(
  alsa-lib base-devel curl file gtk3 libayatana-appindicator librsvg openssl
  patchelf pkgconf webkit2gtk-4.1 wget xdotool
)
ZYPPER_PACKAGES=(
  alsa-devel curl file gcc gcc-c++ libayatana-appindicator3-devel libxdo-devel
  libopenssl-devel make patchelf pkg-config webkit2gtk4.1-devel wget
  gtk3-devel librsvg-devel
)

distro_family() {
  local -a ids=()
  if [[ -r "${OS_RELEASE}" ]]; then
    # shellcheck disable=SC1090
    read -r -a ids <<<"$(. "${OS_RELEASE}" >/dev/null 2>&1 && echo "${ID:-} ${ID_LIKE:-}")"
  fi
  # Match whole tokens, not substrings: `ID=solus` must not match `ol`
  # (Oracle Linux), and `ID_LIKE="..."` contributes only its own tokens.
  local id
  for id in "${ids[@]}"; do
    case "${id}" in
      debian | ubuntu | linuxmint | pop) echo "apt" && return ;;
      fedora | rhel | centos | rocky | almalinux | ol | oracle) echo "dnf" && return ;;
      arch | manjaro | endeavouros) echo "pacman" && return ;;
      suse | sles | sled | opensuse*) echo "zypper" && return ;;
    esac
  done
  echo "unknown"
}

family="$(distro_family)"

install_command() {
  case "${family}" in
    apt)
      echo "sudo apt-get update && sudo apt-get install -y --no-install-recommends ${APT_PACKAGES[*]}"
      ;;
    dnf) echo "sudo dnf install -y ${DNF_PACKAGES[*]}" ;;
    pacman) echo "sudo pacman -S --needed --noconfirm ${PACMAN_PACKAGES[*]}" ;;
    zypper) echo "sudo zypper --non-interactive install ${ZYPPER_PACKAGES[*]}" ;;
    *) echo "(no known package manager for this distribution — see https://tauri.app/start/prerequisites/)" ;;
  esac
}

# Prints the missing modules, one per line, and returns non-zero when any are
# absent or pkg-config itself is unavailable.
collect_missing() {
  local absent=()
  if ! command -v "${PKG_CONFIG}" >/dev/null 2>&1; then
    echo "pkg-config"
    return 0
  fi
  local module
  for module in "${MODULES[@]}"; do
    if ! "${PKG_CONFIG}" --exists "${module}" 2>/dev/null; then
      absent+=("${module}")
    fi
  done
  if ((${#absent[@]} > 0)); then
    printf '%s\n' "${absent[@]}"
  fi
}

report_missing() {
  echo "Missing system libraries required to build the Buzz desktop app:" >&2
  local module
  while IFS= read -r module; do
    echo "  - ${module}" >&2
  done <<<"$1"
  echo "" >&2
  if [[ "${family}" == "unknown" ]]; then
    echo "Install the equivalents for your distribution:" >&2
    echo "  https://tauri.app/start/prerequisites/" >&2
    echo "" >&2
    echo "Then re-run this check." >&2
  else
    echo "Install them with:" >&2
    echo "  just linux-deps" >&2
    echo "" >&2
    echo "Or directly:" >&2
    echo "  $(install_command)" >&2
  fi
}

missing="$(collect_missing)"

if [[ -z "${missing}" ]]; then
  exit 0
fi

if [[ "${MODE}" == "check" ]]; then
  report_missing "${missing}"
  exit 1
fi

# ── --install ────────────────────────────────────────────────────────────────
if [[ "${family}" == "unknown" ]]; then
  report_missing "${missing}"
  echo "" >&2
  echo "Cannot install automatically: unrecognized distribution." >&2
  exit 2
fi

SUDO=()
if [[ "${EUID}" -ne 0 ]]; then
  if ! command -v sudo >/dev/null 2>&1; then
    report_missing "${missing}"
    echo "" >&2
    echo "sudo is not available; re-run as root." >&2
    exit 2
  fi
  SUDO=(sudo)
fi

echo "Installing desktop system dependencies (${family})..."
case "${family}" in
  apt)
    "${SUDO[@]}" apt-get update
    "${SUDO[@]}" apt-get install -y --no-install-recommends "${APT_PACKAGES[@]}"
    ;;
  dnf) "${SUDO[@]}" dnf install -y "${DNF_PACKAGES[@]}" ;;
  pacman) "${SUDO[@]}" pacman -S --needed --noconfirm "${PACMAN_PACKAGES[@]}" ;;
  zypper) "${SUDO[@]}" zypper --non-interactive install "${ZYPPER_PACKAGES[@]}" ;;
esac

missing="$(collect_missing)"
if [[ -n "${missing}" ]]; then
  report_missing "${missing}"
  exit 1
fi

echo "Desktop system dependencies are installed."
