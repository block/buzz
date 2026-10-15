#!/usr/bin/env bash

install_appimage_xdg_open() {
  local appdir="$1"
  local wrapper_source="$2"
  local xdg_open="$appdir/usr/bin/xdg-open"

  if [[ ! -f "$xdg_open" ]]; then
    echo "Error: bundled usr/bin/xdg-open not found - bundler layout changed; update fix-appimage.sh" >&2
    return 1
  fi
  if [[ -e "$xdg_open.appimage" ]]; then
    echo "Error: usr/bin/xdg-open.appimage already exists - wrapper already installed?" >&2
    return 1
  fi

  mv "$xdg_open" "$xdg_open.appimage"
  install -m 755 "$wrapper_source" "$xdg_open"
}
