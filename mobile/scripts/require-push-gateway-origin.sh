#!/bin/sh
set -eu

gateway_origin=
has_dart_define=false
old_ifs=$IFS
IFS=','
for encoded in ${DART_DEFINES:-}; do
  decoded=$(printf '%s' "$encoded" | base64 --decode 2>/dev/null || printf '%s' "$encoded" | base64 -D 2>/dev/null || true)
  case "$decoded" in
    BUZZ_PUSH_GATEWAY_URL=*)
      if [ "$has_dart_define" = true ]; then
        echo "error: BUZZ_PUSH_GATEWAY_URL must be supplied at most once." >&2
        exit 1
      fi
      has_dart_define=true
      gateway_origin=${decoded#BUZZ_PUSH_GATEWAY_URL=}
      ;;
  esac
done
IFS=$old_ifs

if [ "$has_dart_define" = false ] && [ -n "${BUZZ_PUSH_GATEWAY_URL:-}" ]; then
  gateway_origin=$BUZZ_PUSH_GATEWAY_URL
  encoded=$(printf '%s' "BUZZ_PUSH_GATEWAY_URL=$gateway_origin" | base64 | tr -d '\n')
  DART_DEFINES=${DART_DEFINES:+$DART_DEFINES,}$encoded
  export DART_DEFINES
fi

if [ "$has_dart_define" = false ] && [ -z "$gateway_origin" ]; then
  # An unconfigured artifact deliberately has no push capability.
  return 0 2>/dev/null || exit 0
fi

if [ -n "${SRCROOT:-}" ]; then
  script_dir=$SRCROOT/../scripts
else
  script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
fi
dart_bin=${FLUTTER_ROOT:+$FLUTTER_ROOT/bin/dart}
if [ -z "$dart_bin" ] || [ ! -x "$dart_bin" ]; then
  dart_bin=$(command -v dart || true)
fi
if [ -z "$dart_bin" ]; then
  echo "error: Dart is required to validate BUZZ_PUSH_GATEWAY_URL." >&2
  exit 1
fi
case ${CONFIGURATION:-Release} in
  Debug*)
    "$dart_bin" "$script_dir/validate_push_gateway_origin.dart" "$gateway_origin"
    ;;
  *)
    "$dart_bin" "$script_dir/validate_push_gateway_origin.dart" --require-https "$gateway_origin"
    ;;
esac
