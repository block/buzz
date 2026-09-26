#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$script_dir/appimage-xdg-open-install.sh"

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

appdir="$workdir/App Dir"
appdir_alias="$workdir/AppDir-alias"
host_bin="$workdir/host bin"
tool_bin="$workdir/tool-bin"
capture_dir="$workdir/capture"
mkdir -p "$appdir/usr/bin" "$host_bin" "$tool_bin" "$capture_dir"
ln -s "$appdir" "$appdir_alias"

cat > "$appdir/usr/bin/xdg-open" <<'CAPTURE_OPENER'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$@" > "$CAPTURE_DIR/args"
env | LC_ALL=C sort > "$CAPTURE_DIR/env"
CAPTURE_OPENER
chmod +x "$appdir/usr/bin/xdg-open"
cp "$appdir/usr/bin/xdg-open" "$host_bin/xdg-open"

url='https://example.com/auth/login?returnTo=http%3A%2F%2F127.0.0.1%3A1234%2Fcallback&state=safe-test'
contaminated_env=(
  "PATH=$appdir_alias/usr/bin:$host_bin:/usr/bin:/bin"
  "CAPTURE_DIR=$capture_dir"
  "APPDIR=$appdir"
  "APPIMAGE=$workdir/Buzz.AppImage"
  "LD_LIBRARY_PATH=$appdir_alias/usr/lib:/host/lib"
  "XDG_DATA_DIRS=$appdir_alias/usr/share:/host/share one:/host/share-two"
  "GIO_EXTRA_MODULES=$appdir/usr/lib/gio/modules:/host/gio"
  "GTK_PATH=$appdir/usr/lib/gtk:/host/gtk"
  "QT_PLUGIN_PATH=$appdir/usr/plugins:/host/qt"
  "PYTHONPATH=$appdir/usr/python:/host/python"
  "GSETTINGS_SCHEMA_DIR=$appdir/usr/share/glib-2.0/schemas"
  "GTK_DATA_PREFIX=$appdir/usr"
  "GTK_EXE_PREFIX=$appdir/usr"
  "GTK_IM_MODULE_FILE=$appdir/usr/lib/gtk.immodules"
  "GDK_PIXBUF_MODULE_FILE=$appdir/usr/lib/gdk-pixbuf.loaders"
  "GDK_BACKEND=x11"
)

# Before installation, packaged PATH resolves the bundled opener and every
# AppRun value reaches it. This assertion proves the fixture detects the bug.
env "${contaminated_env[@]}" xdg-open "$url"
[[ "$(<"$capture_dir/args")" == "$url" ]]
grep -Fqx "APPDIR=$appdir" "$capture_dir/env"
grep -Fqx "XDG_DATA_DIRS=$appdir_alias/usr/share:/host/share one:/host/share-two" \
  "$capture_dir/env"
rm -f "$capture_dir/args" "$capture_dir/env"

install_appimage_xdg_open "$appdir" "$script_dir/appimage-xdg-open"
[[ -x "$appdir/usr/bin/xdg-open" ]]
[[ -x "$appdir/usr/bin/xdg-open.appimage" ]]
cmp "$script_dir/appimage-xdg-open" "$appdir/usr/bin/xdg-open"

# Exercise the installed boundary through the same packaged PATH lookup.
env "${contaminated_env[@]}" xdg-open "$url"
[[ "$(<"$capture_dir/args")" == "$url" ]]

for variable in APPDIR APPIMAGE GSETTINGS_SCHEMA_DIR GTK_DATA_PREFIX \
  GTK_EXE_PREFIX GTK_IM_MODULE_FILE GDK_PIXBUF_MODULE_FILE GDK_BACKEND; do
  if grep -q "^$variable=" "$capture_dir/env"; then
    echo "expected $variable to be unset" >&2
    exit 1
  fi
done

grep -Fqx "LD_LIBRARY_PATH=/host/lib" "$capture_dir/env"
grep -Fqx "XDG_DATA_DIRS=/host/share one:/host/share-two" "$capture_dir/env"
grep -Fqx "GIO_EXTRA_MODULES=/host/gio" "$capture_dir/env"
grep -Fqx "GTK_PATH=/host/gtk" "$capture_dir/env"
grep -Fqx "QT_PLUGIN_PATH=/host/qt" "$capture_dir/env"
grep -Fqx "PYTHONPATH=/host/python" "$capture_dir/env"
grep -Fqx "PATH=$host_bin:/usr/bin:/bin" "$capture_dir/env"

# Host-owned single paths and backend choices must survive cleanup.
rm -f "$capture_dir/args" "$capture_dir/env"
env "${contaminated_env[@]}" \
  GTK_DATA_PREFIX=/host/gtk-data \
  GTK_EXE_PREFIX=/host/gtk-exe \
  GTK_IM_MODULE_FILE=/host/gtk.immodules \
  GDK_PIXBUF_MODULE_FILE=/host/gdk-pixbuf.loaders \
  GSETTINGS_SCHEMA_DIR=/host/schemas \
  GDK_BACKEND=wayland \
  xdg-open "$url"
for expected in \
  'GTK_DATA_PREFIX=/host/gtk-data' \
  'GTK_EXE_PREFIX=/host/gtk-exe' \
  'GTK_IM_MODULE_FILE=/host/gtk.immodules' \
  'GDK_PIXBUF_MODULE_FILE=/host/gdk-pixbuf.loaders' \
  'GSETTINGS_SCHEMA_DIR=/host/schemas' \
  'GDK_BACKEND=wayland'; do
  grep -Fqx "$expected" "$capture_dir/env"
done

# If no host opener exists, the retained bundled opener remains executable.
rm -f "$capture_dir/args" "$capture_dir/env"
ln -s /usr/bin/bash "$tool_bin/bash"
ln -s /usr/bin/dirname "$tool_bin/dirname"
ln -s /usr/bin/env "$tool_bin/env"
ln -s /usr/bin/readlink "$tool_bin/readlink"
ln -s /usr/bin/realpath "$tool_bin/realpath"
ln -s /usr/bin/sort "$tool_bin/sort"
PATH="$appdir_alias/usr/bin:$tool_bin" CAPTURE_DIR="$capture_dir" \
  "$appdir/usr/bin/xdg-open" "$url"
[[ "$(<"$capture_dir/args")" == "$url" ]]

printf 'appimage-xdg-open integration tests passed\n'
