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

echo "==> Symlinking system GStreamer plugin directory"
# On distros without the Debian multiarch layout (e.g. Arch), this symlink
# dangles — GStreamer then falls back to its default plugin discovery, which
# is a safe degradation (unlike the original empty in-bundle dir).
rm -rf "$LIBDIR/gstreamer-1.0"
ln -s "/usr/lib/$MULTIARCH/gstreamer-1.0" "$LIBDIR/gstreamer-1.0"

# WebKitGTK 2.52 Skia's GPU path goes through Vulkan/radv. On non-conformant
# AMD RDNA4 (gfx1200) that path paints nothing — transparent ghost window —
# while WEBKIT_DISABLE_DMABUF_RENDERER does not help (#2643). Prefer Skia CPU
# raster for AppImage launches; operators can override with =0.
echo "==> Preferring WebKitGTK Skia CPU rendering for AppImage launches"
HOOK_DIR="$WORKDIR/squashfs-root/apprun-hooks"
mkdir -p "$HOOK_DIR"
cat > "$HOOK_DIR/99-buzz-webkit-skia-cpu.sh" <<'EOF'
# Allow operators to override (e.g. WEBKIT_SKIA_ENABLE_CPU_RENDERING=0) if needed.
export WEBKIT_SKIA_ENABLE_CPU_RENDERING="${WEBKIT_SKIA_ENABLE_CPU_RENDERING:-1}"
EOF
if [[ -f "$WORKDIR/squashfs-root/AppRun" ]] \
  && ! grep -q 'WEBKIT_SKIA_ENABLE_CPU_RENDERING' "$WORKDIR/squashfs-root/AppRun"; then
  if ! grep -q 'apprun-hooks' "$WORKDIR/squashfs-root/AppRun"; then
    tmp_apprun="$(mktemp)"
    {
      if head -n1 "$WORKDIR/squashfs-root/AppRun" | grep -q '^#!'; then
        head -n1 "$WORKDIR/squashfs-root/AppRun"
        echo 'export WEBKIT_SKIA_ENABLE_CPU_RENDERING="${WEBKIT_SKIA_ENABLE_CPU_RENDERING:-1}"'
        tail -n +2 "$WORKDIR/squashfs-root/AppRun"
      else
        echo 'export WEBKIT_SKIA_ENABLE_CPU_RENDERING="${WEBKIT_SKIA_ENABLE_CPU_RENDERING:-1}"'
        cat "$WORKDIR/squashfs-root/AppRun"
      fi
    } > "$tmp_apprun"
    mv "$tmp_apprun" "$WORKDIR/squashfs-root/AppRun"
    chmod +x "$WORKDIR/squashfs-root/AppRun"
  fi
fi

# Fedora's COLRv1 Noto Color Emoji trips an assertion in WebKitGTK/Skia and
# aborts the AppImage after a blank window (#2548). Reject that family so
# emoji fall back to a non-COLRv1 face (or to monochrome glyphs).
echo "==> Rejecting COLRv1 system color-emoji fonts for AppImage launches"
FC_DIR="$WORKDIR/squashfs-root/usr/etc/fonts"
mkdir -p "$FC_DIR"
cat > "$FC_DIR/fonts.conf" <<'EOF'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <!-- Keep the host fontconfig, then drop COLRv1 color-emoji faces that crash
       bundled WebKitGTK/Skia on Fedora (#2548). -->
  <include ignore_missing="yes">/etc/fonts/fonts.conf</include>
  <selectfont>
    <rejectfont>
      <pattern>
        <patelt name="family"><string>Noto Color Emoji</string></patelt>
      </pattern>
    </rejectfont>
  </selectfont>
</fontconfig>
EOF
cat > "$HOOK_DIR/98-buzz-fontconfig-no-colrv1.sh" <<'EOF'
# Prefer the AppImage fontconfig that rejects COLRv1 Noto Color Emoji (#2548).
# Operators can point FONTCONFIG_FILE elsewhere to override.
if [ -z "${FONTCONFIG_FILE:-}" ] && [ -n "${APPDIR:-}" ] && [ -f "$APPDIR/usr/etc/fonts/fonts.conf" ]; then
  export FONTCONFIG_FILE="$APPDIR/usr/etc/fonts/fonts.conf"
fi
EOF
if [[ -f "$WORKDIR/squashfs-root/AppRun" ]] \
  && ! grep -q 'buzz-fontconfig-no-colrv1\|FONTCONFIG_FILE=.*usr/etc/fonts' "$WORKDIR/squashfs-root/AppRun"; then
  if ! grep -q 'apprun-hooks' "$WORKDIR/squashfs-root/AppRun"; then
    tmp_apprun="$(mktemp)"
    {
      if head -n1 "$WORKDIR/squashfs-root/AppRun" | grep -q '^#!'; then
        head -n1 "$WORKDIR/squashfs-root/AppRun"
        echo 'if [ -z "${FONTCONFIG_FILE:-}" ] && [ -n "${APPDIR:-}" ] && [ -f "$APPDIR/usr/etc/fonts/fonts.conf" ]; then export FONTCONFIG_FILE="$APPDIR/usr/etc/fonts/fonts.conf"; fi'
        tail -n +2 "$WORKDIR/squashfs-root/AppRun"
      else
        echo 'if [ -z "${FONTCONFIG_FILE:-}" ] && [ -n "${APPDIR:-}" ] && [ -f "$APPDIR/usr/etc/fonts/fonts.conf" ]; then export FONTCONFIG_FILE="$APPDIR/usr/etc/fonts/fonts.conf"; fi'
        cat "$WORKDIR/squashfs-root/AppRun"
      fi
    } > "$tmp_apprun"
    mv "$tmp_apprun" "$WORKDIR/squashfs-root/AppRun"
    chmod +x "$WORKDIR/squashfs-root/AppRun"
  fi
fi

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
