#!/usr/bin/env bash
# fix-appimage.sh — Remove infra libs from a Tauri-produced AppImage that crash
# on Mesa 25+ / GLib 2.88 distros (Ubuntu 26.04, Fedora 42+, etc.).
#
# Usage: fix-appimage.sh <path-to.AppImage>
#
# Set TAURI_SIGNING_PRIVATE_KEY / TAURI_SIGNING_PRIVATE_KEY_PASSWORD to
# re-sign after repacking (CI release builds). Without them the script
# repacks but skips signing, which is fine for local testing.
#
# Root cause — three interlocking failures (upstream: https://github.com/tauri-apps/tauri/issues/15665):
#
#  1. EGL crash: linuxdeploy bundles libwayland-client.so.0 (1.22) alongside
#     the app. Mesa 25's libEGL calls the bundled version at runtime; the version
#     skew causes eglGetDisplay to return EGL_BAD_PARAMETER under Wayland, which
#     WebKitWebProcess treats as fatal and aborts before the window ever appears.
#
#  2. GStreamer crash: linuxdeploy also bundles libgst*.so* (GStreamer core libs).
#     AppRun unconditionally sets GST_PLUGIN_SYSTEM_PATH_1_0 to a dir inside the
#     AppImage that the bundler never populates (bundleMediaFramework is false by
#     default), so GStreamer's plugin discovery yields an empty registry. The
#     "GStreamer element appsink not found" error kills the render process; as a
#     side effect the broken run poisons ~/.cache/gstreamer-1.0/registry.x86_64.bin.
#
#  3. WebKit helper mismatch (latent): the bundled WebKit helpers
#     (WebKitNetworkProcess/WebKitWebProcess) have RUNPATH=$ORIGIN only, and
#     linuxdeploy string-patches /usr -> ././ inside libwebkit2gtk so the helper
#     dir is resolved relative to the process cwd. AppRun's chdir($APPDIR/usr)
#     makes this work; any launch that bypasses AppRun (extracted-AppDir usage,
#     repack workflows, dbus/systemd activation with cwd=/) resolves the helpers
#     wrong -- spawning nothing, dying on unresolved bundled libs, or spawning
#     the system helpers -- and the window never appears.
#
# Fix: remove the offending libs so the app uses the system copies (which are
# newer and ABI-compatible on any distro shipping glib >= 2.72 / Ubuntu 22.04+),
# and symlink the system GStreamer plugin directory so discovery works correctly.
# No tauri.conf.json knob can do this — bundle.linux.appimage only exposes
# bundleMediaFramework, files (copy-only, no remove/symlink), and bundleXdgOpen.

set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "Usage: fix-appimage.sh <path-to.AppImage>" >&2
  exit 1
fi

APPIMAGE_ABS="$(realpath "$1")"
APPIMAGE_DIR="$(dirname "$APPIMAGE_ABS")"
APPIMAGE_NAME="$(basename "$APPIMAGE_ABS")"

if [[ ! -f "$APPIMAGE_ABS" ]]; then
  echo "Error: file not found: $APPIMAGE_ABS" >&2
  exit 1
fi

# Locate the desktop/ directory (this script lives at desktop/scripts/).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Detect multiarch triplet for GStreamer plugin path.
case "$(uname -m)" in
  x86_64)  MULTIARCH="x86_64-linux-gnu" ;;
  aarch64) MULTIARCH="aarch64-linux-gnu" ;;
  *)
    echo "Error: unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

echo "==> Extracting $APPIMAGE_NAME"
(cd "$WORKDIR" && APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGE_ABS" --appimage-extract)

LIBDIR="$WORKDIR/squashfs-root/usr/lib"

echo "==> Removing infra libs that conflict with system Mesa / GLib / GStreamer"
rm -f \
  "$LIBDIR"/libwayland-client.so* \
  "$LIBDIR"/libwayland-cursor.so* \
  "$LIBDIR"/libwayland-egl.so* \
  "$LIBDIR"/libwayland-server.so* \
  "$LIBDIR"/libglib-2.0.so* \
  "$LIBDIR"/libgio-2.0.so* \
  "$LIBDIR"/libgobject-2.0.so* \
  "$LIBDIR"/libgmodule-2.0.so* \
  "$LIBDIR"/libmount.so* \
  "$LIBDIR"/libblkid.so* \
  "$LIBDIR"/libselinux.so* \
  "$LIBDIR"/libpcre2-8.so* \
  "$LIBDIR"/libgst*.so* \
  "$LIBDIR"/libzstd.so* \
  "$LIBDIR"/libelf.so* \
  "$LIBDIR"/libffi.so*

echo "==> Symlinking system GStreamer plugin directory"
rm -rf "$LIBDIR/gstreamer-1.0"
ln -s "/usr/lib/$MULTIARCH/gstreamer-1.0" "$LIBDIR/gstreamer-1.0"

echo "==> Repacking AppImage"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$(uname -m)" appimagetool \
  "$WORKDIR/squashfs-root" "$APPIMAGE_ABS"

# Re-sign after repack so the updater can verify the artifact.
# Tauri 2.11 with createUpdaterArtifacts=true produces two possible formats:
#   New: <name>.AppImage + <name>.AppImage.sig   (sign the AppImage directly)
#   Old: <name>.AppImage.tar.gz + .tar.gz.sig    (tar-wrapped, then signed)
# We handle both: always re-sign the AppImage; if a .tar.gz sibling exists
# alongside it, recreate it from the freshly repacked AppImage and re-sign that.
if [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  echo "==> Re-signing AppImage"
  (cd "$DESKTOP_DIR" && pnpm tauri signer sign \
    ${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:+--password "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD"} \
    "$APPIMAGE_ABS")

  TARBALL="$APPIMAGE_ABS.tar.gz"
  if [[ -f "$TARBALL" ]]; then
    echo "==> Recreating updater archive $TARBALL"
    tar -czf "$TARBALL" -C "$APPIMAGE_DIR" "$APPIMAGE_NAME"
    echo "==> Re-signing updater archive"
    (cd "$DESKTOP_DIR" && pnpm tauri signer sign \
      ${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:+--password "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD"} \
      "$TARBALL")
  fi
else
  echo "==> TAURI_SIGNING_PRIVATE_KEY not set — skipping signing (local build)"
fi

echo "==> Done: $APPIMAGE_ABS"
