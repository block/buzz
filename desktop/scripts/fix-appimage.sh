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
# Set APPIMAGETOOL_RUNTIME_FILE to a pre-downloaded AppImage type2 runtime to
# avoid appimagetool fetching one from its mutable `continuous` tag (CI pins
# this; unset is fine for local testing).
#
# Root cause — three interlocking failures (upstream: https://github.com/tauri-apps/tauri/issues/15665):
#
#  1. EGL crash: linuxdeploy bundles libwayland-client.so.0 (1.22) alongside
#     the app. Mesa 25's libEGL calls the bundled version at runtime; the version
#     skew causes eglGetDisplay to return EGL_BAD_PARAMETER under Wayland, which
#     WebKitWebProcess treats as fatal and aborts before the window ever appears.
#
#  2. GStreamer crash: linuxdeploy's compiled AppRun.wrapped force-sets
#     GST_PLUGIN_SYSTEM_PATH_1_0 to $APPDIR/usr/lib/gstreamer-1.0 -- a dir the
#     bundler never populates (bundleMediaFramework is off, and we strip the
#     bundled libgst* core below to use the host's). Crucially, once that variable
#     is set it *replaces* GStreamer's compiled-in default search path rather than
#     adding to it, so the app finds ZERO plugins on every distro:
#     "GStreamer element appsink not found" kills the WebKitWebProcess and the
#     window never paints. An earlier revision of this script hid the failure on
#     Debian only by symlinking usr/lib/gstreamer-1.0 to the Debian multiarch dir
#     (/usr/lib/x86_64-linux-gnu/gstreamer-1.0); that symlink dangles on Arch and
#     Fedora, and the "safe fallback to default discovery" it assumed does not
#     exist -- a set GST_PLUGIN_SYSTEM_PATH_1_0 disables the default. A broken run
#     also poisons ~/.cache/gstreamer-1.0/registry.x86_64.bin.
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
# Fix: (a) remove the offending libs so the app uses the system copies (newer and
# ABI-compatible on any distro shipping glib >= 2.72 / Ubuntu 22.04+), and
# (b) install a launcher shim in front of the app binary that strips the
# bundle-pointing GST_PLUGIN_* overrides AppRun.wrapped injects, letting the host
# GStreamer resolve plugins via its own default path (correct on Debian, Arch, and
# Fedora alike). The shim has to run *after* AppRun.wrapped: the wrapper rewrites
# the variable last -- after every apprun-hook -- so any value set before it is
# discarded (verified empirically; a runtime GST_PLUGIN_SYSTEM_PATH_1_0 passed
# into the AppImage does not survive). No tauri.conf.json knob can do this --
# bundle.linux.appimage only exposes bundleMediaFramework, files (copy-only, no
# remove/symlink), and bundleXdgOpen.

set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "Usage: fix-appimage.sh <path-to.AppImage>" >&2
  exit 1
fi

if [[ ! -f "$1" ]]; then
  echo "Error: file not found: $1" >&2
  exit 1
fi

APPIMAGE_ABS="$(realpath "$1")"
APPIMAGE_DIR="$(dirname "$APPIMAGE_ABS")"
APPIMAGE_NAME="$(basename "$APPIMAGE_ABS")"

# Locate the desktop/ directory (this script lives at desktop/scripts/).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

echo "==> Extracting $APPIMAGE_NAME"
(cd "$WORKDIR" && APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGE_ABS" --appimage-extract)

LIBDIR="$WORKDIR/squashfs-root/usr/lib"

# Guard against a bundler layout change: if the primary offending lib is not
# where we expect it, the rm globs below would silently no-op and we'd ship
# an unfixed artifact. Fail loudly instead so a tauri/linuxdeploy upgrade
# that changes the bundled lib set gets noticed here, not by users.
if ! compgen -G "$LIBDIR/libwayland-client.so*" > /dev/null; then
  echo "Error: libwayland-client not found in $LIBDIR — bundler layout changed; update fix-appimage.sh" >&2
  exit 1
fi

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

echo "==> Installing GStreamer launcher shim on the app binary"
# AppRun.wrapped force-sets GST_PLUGIN_SYSTEM_PATH_1_0 (and the 0.10-era
# GST_PLUGIN_SYSTEM_PATH) to $APPDIR/usr/lib/gstreamer-1.0 — a dir we bundle no
# plugins into. Because a set path *replaces* GStreamer's default instead of
# extending it, the app finds zero plugins on any distro and WebKit aborts. The
# wrapper rewrites the variable after every apprun-hook, so the only place to undo
# it is a shim between AppRun.wrapped and the real binary. First confirm the
# wrapper still injects the override; if a tauri/linuxdeploy bump drops it, the
# shim becomes a harmless no-op, but we want a human to re-verify rather than
# silently ship — so fail loudly (mirrors the libwayland guard above).
APPRUN_WRAPPED="$WORKDIR/squashfs-root/AppRun.wrapped"
if ! grep -aq "GST_PLUGIN_SYSTEM_PATH_1_0" "$APPRUN_WRAPPED"; then
  echo "Error: AppRun.wrapped no longer references GST_PLUGIN_SYSTEM_PATH_1_0 — GStreamer path injection changed; re-verify fix-appimage.sh" >&2
  exit 1
fi

APP_BIN="$WORKDIR/squashfs-root/usr/bin/buzz-desktop"
if [[ ! -f "$APP_BIN" ]]; then
  echo "Error: app binary usr/bin/buzz-desktop not found — bundler layout changed; update fix-appimage.sh" >&2
  exit 1
fi
if [[ -e "$APP_BIN.bin" ]]; then
  echo "Error: usr/bin/buzz-desktop.bin already exists — shim already installed?" >&2
  exit 1
fi

# The real binary moves aside; buzz-desktop becomes a shim AppRun.wrapped execs.
mv "$APP_BIN" "$APP_BIN.bin"
cat > "$APP_BIN" <<'SHIM'
#!/usr/bin/env bash
# GStreamer shim installed by desktop/scripts/fix-appimage.sh.
#
# linuxdeploy's AppRun.wrapped force-sets GST_PLUGIN_SYSTEM_PATH_1_0 to an empty
# in-bundle dir ($APPDIR/usr/lib/gstreamer-1.0). A set path *replaces* the host's
# default GStreamer search path, so the app finds zero plugins and WebKit aborts
# (blank window). Drop the bundle-pointing GST_PLUGIN_* overrides so the system
# GStreamer — which we use, having removed the bundled core libs — resolves
# plugins via its own default path on any distro. Values that don't point into
# this AppImage are the user's own and are preserved.
here="$(dirname "$(readlink -f "$0")")"
appdir="$(readlink -f "$here/../..")"
for var in GST_PLUGIN_SYSTEM_PATH_1_0 GST_PLUGIN_SYSTEM_PATH \
           GST_PLUGIN_PATH_1_0 GST_PLUGIN_PATH \
           GST_PLUGIN_SCANNER GST_PLUGIN_SCANNER_1_0; do
  val="${!var-}"
  if [[ -n "$val" && "$val" == *"$appdir"* ]]; then
    unset "$var"
  fi
done
exec "$here/buzz-desktop.bin" "$@"
SHIM
chmod +x "$APP_BIN"

echo "==> Repacking AppImage"
# Pass a pinned type2 runtime when provided (CI sets APPIMAGETOOL_RUNTIME_FILE);
# without it appimagetool downloads the runtime from its mutable `continuous`
# tag at repack time — acceptable for local testing, not for release builds.
RUNTIME_ARGS=()
if [[ -n "${APPIMAGETOOL_RUNTIME_FILE:-}" ]]; then
  RUNTIME_ARGS=(--runtime-file "$APPIMAGETOOL_RUNTIME_FILE")
fi
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$(uname -m)" appimagetool \
  "${RUNTIME_ARGS[@]}" \
  "$WORKDIR/squashfs-root" "$APPIMAGE_ABS"

# Re-sign after repack so the updater can verify the artifact.
# Tauri 2.11 with createUpdaterArtifacts=true produces two possible formats:
#   New: <name>.AppImage + <name>.AppImage.sig   (sign the AppImage directly)
#   Old: <name>.AppImage.tar.gz + .tar.gz.sig    (tar-wrapped, then signed)
# We handle both: always re-sign the AppImage; if a .tar.gz sibling exists
# alongside it, recreate it from the freshly repacked AppImage and re-sign that.
# Our release config pins createUpdaterArtifacts: true (build-release-config.mjs),
# so the tar.gz branch is dead in CI today — kept deliberately because the
# workflow's artifact-locate step prefers a tar.gz when one exists; dropping
# this branch could publish a stale tarball containing the unfixed AppImage.
if [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  # `tauri signer sign` reads TAURI_SIGNING_PRIVATE_KEY and
  # TAURI_SIGNING_PRIVATE_KEY_PASSWORD from the environment (same as the
  # macOS jobs in release.yml) — never pass the password via argv, where
  # it would be visible in /proc/<pid>/cmdline.
  echo "==> Re-signing AppImage"
  (cd "$DESKTOP_DIR" && pnpm tauri signer sign "$APPIMAGE_ABS")

  TARBALL="$APPIMAGE_ABS.tar.gz"
  if [[ -f "$TARBALL" ]]; then
    echo "==> Recreating updater archive $TARBALL"
    tar -czf "$TARBALL" -C "$APPIMAGE_DIR" "$APPIMAGE_NAME"
    echo "==> Re-signing updater archive"
    (cd "$DESKTOP_DIR" && pnpm tauri signer sign "$TARBALL")
  fi
else
  echo "==> TAURI_SIGNING_PRIVATE_KEY not set — skipping signing (local build)"
fi

echo "==> Done: $APPIMAGE_ABS"
