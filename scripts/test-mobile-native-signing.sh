#!/usr/bin/env bash
# Exercise the production C adapter from Dart on the macOS native CI runner.
set -euo pipefail
cd "$(dirname "$0")/.."
. ./bin/activate-hermit
native_test_dir=$(mktemp -d)
trap 'rm -rf "$native_test_dir"' EXIT
source_dir=mobile/ios/BuzzPushKit/Sources/CSecp256k1
clang -O2 -dynamiclib -DENABLE_MODULE_EXTRAKEYS -DENABLE_MODULE_SCHNORRSIG \
  -framework Security \
  "$source_dir/buzz_signer.c" \
  "$source_dir/vendor/src/secp256k1.c" \
  "$source_dir/vendor/src/precomputed_ecmult.c" \
  "$source_dir/vendor/src/precomputed_ecmult_gen.c" \
  -o "$native_test_dir/libbuzz_signer.dylib"
cd mobile
BUZZ_SIGNER_LIBRARY="$native_test_dir/libbuzz_signer.dylib" \
  flutter test test/shared/crypto/event_signer_test.dart
