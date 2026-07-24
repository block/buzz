#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
developer_dir=${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
target=${1:-$(rustc -vV | sed -n 's|host: ||p')}

case "$target" in
  aarch64-apple-darwin) arch=arm64 ;;
  x86_64-apple-darwin) arch=x86_64 ;;
  *)
    echo "Apple-input helper can only be built for a macOS target; got $target" >&2
    exit 2
    ;;
esac

if [[ "$developer_dir" != "/Applications/Xcode.app/Contents/Developer" || ! -d "$developer_dir" ]]; then
  echo "DEVELOPER_DIR must be /Applications/Xcode.app/Contents/Developer" >&2
  exit 2
fi

derived_data="$repo_root/.cache/apple-inputs/$target"
project="$repo_root/desktop/apple-inputs/BuzzAppleInputs.xcodeproj"
destination="$repo_root/desktop/src-tauri/binaries/buzz-apple-inputs-$target"

DEVELOPER_DIR="$developer_dir" xcodebuild \
  -project "$project" \
  -scheme BuzzAppleInputs \
  -configuration Release \
  -derivedDataPath "$derived_data" \
  -destination "generic/platform=macOS" \
  ARCHS="$arch" \
  ONLY_ACTIVE_ARCH=YES \
  CODE_SIGNING_ALLOWED=NO \
  build

product="$derived_data/Build/Products/Release/BuzzAppleInputs"
[[ -x "$product" ]] || {
  echo "Apple-input helper build did not produce an executable: $product" >&2
  exit 1
}
mkdir -p "$(dirname "$destination")"
install -m 0755 "$product" "$destination"
echo "Apple-input helper bundled for $target"
