# Native packages

Local Dart packages that wrap a native C library through `dart:ffi`. Each one
compiles its library from vendored source with a Dart build hook
(`hook/build.dart`), so Flutter's native assets support builds it for every
target (iOS, Android, and the host for `flutter test`). No CocoaPods, Gradle,
or CMake plugin wiring is involved.

## Layout

```
packages/<name>/
  pubspec.yaml          # depends on hooks, code_assets, native_toolchain_c, ffi
  hook/build.dart       # CBuilder.library over the vendored sources
  third_party/<lib>/    # pinned upstream subset + VENDORED.md (tag, sha256)
  src/*.c               # optional shims when the upstream API needs adapting
  lib/src/bindings.dart # @Native leaf bindings, @DefaultAsset = the hook's assetName
  lib/<name>.dart       # small typed Dart API that owns memory and validation
  test/                 # upstream test vectors where they exist
```

## Adding a library

1. Vendor the smallest source subset from a tagged upstream release and record
   the tag and tarball sha256 in `third_party/<lib>/VENDORED.md`. Do not edit
   vendored files.
2. Mirror the upstream build system's compile definitions in `hook/build.dart`.
3. Bind only the symbols the app needs. Mark calls `isLeaf: true` unless they
   call back into Dart, and keep callbacks in C shims so the bindings work in
   any isolate.
4. Add the package to `mobile/pubspec.yaml` as a `path:` dependency.

## Packages

- `buzz_secp256k1`: bitcoin-core/libsecp256k1 (BIP-340 Schnorr, ECDH).
